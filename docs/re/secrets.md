# Dragon Ball Z: Super Saiya Densetsu - hidden and conditional events

Purpose: input for automated test play and for building save states at story points.
Method: text sources only (guides, wikis, forum posts, TAS notes, code lists). Nothing below was
run in an emulator yet. Every claim carries a certainty tag, so the test runs know what to verify first.

Update: the events were then tried in the emulator with crafted saves; what was reached, the save recipes
and what was not are in `secrets-coverage.md`, and the story engine they run on is in `data/scenes.md`.
The text below is the original, source-only research and is not corrected.

Build note: the repo targets the Klepto v1.02 English patch applied to the Japanese Rev 1 ROM
(`crates/ssd-app/src/main.rs`, the `--ips dbz102.ips` option). Most sources describe the Japanese
original (TASVideos uses JP V1.0). Build-dependent points are flagged, and section 10 lists them.

Certainty tags:
- [A] two or more independent sources agree, and at least one is a detailed guide or wiki.
- [B] one detailed source, or two sources that differ in small details.
- [C] anecdote, inference, or conflicting sources. Test before relying on it.

Source keys (full list with URLs in section 11): RPGC = RPG Classics "Legend of the Super Saiyan"
shrine (English walkthrough, cheats, codes, items, bosses); LUC = Lucifer's Hobby (Japanese fan site,
per-map, per-character, per-enemy and card-table notes); JAWIKI = Japanese Wikipedia article (game
system and footnotes); FANDOM = Dragon Ball Wiki article; TAS = TASVideos runs and submission notes;
CH = Critical Hits tips page (Portuguese); GB = Giant Bomb "Secrets" section; KZ = Kanzenshuu forum;
ALM = Almar's Guides code lists.

Internal references: RAM offsets below are cross-checked against `docs/re/data/characters.md`
(party records at $7E:0B00) and `docs/re/data/cards.md` (card hand at $7E:0141).

---

## 0. Story progression outline

Fourteen story maps. English names are from the Dragon Ball Wiki; Japanese names are from LUC.

| # | Map (EN / JP) | Where | You control (party) | Notes |
|---|---|---|---|---|
| 1 | Earth Assault! Raditz! / 強襲！ラディッツ！ | Kame House, mountains, cave | Goku, Piccolo | Needs the Gohan card (S-01). Raditz is forced at Level 4 (S-03). Goku dies (S-04). |
| 2 | Earth - Gohan's training / 悟飯の修行 | Earth | Gohan, Piccolo | Practice fight against Piccolo. |
| 3 | King Kai's Planet - Goku's training / 悟空の修行 | King Kai's planet (afterlife) | Goku (dead) | Clear the gravity trials, then King Kai. The speed sets Goku's return (S-05, S-07). |
| 4 | Earth - Vegeta attacks / ベジータ襲来 | Earth, south and west of start | Gohan, Piccolo, recruits | Recruit window (S-08 to S-11). Piccolo dies in the Nappa fight (S-12). Vegeta duel (S-15). |
| 5 | Planet Namek - Frieza appears / フリーザ登場！ | Namek village, caves, ship | Gohan, surviving recruits, Dende, Cargo; Maima and Tsumuri briefly | Dende and Cargo rescues, 2-star and 6-star balls, Cui, Dodoria (TR-01 to TR-06). |
| 6 | Earth - Goku's training 1 / 超重力修行 (spaceship) | Spaceship gravity room | Goku | Gravity trials 10x to 100x (TR-08). |
| 7 | King Kai's planet - Piccolo's training / ピッコロの修行 | Afterlife | Piccolo (dead) | Piccolo vs Nappa and Raditz (TR-09). |
| 8 | Planet Namek - The original Elder / 最長老の元へ | Namek caves, Guru's house | Gohan, recruits (if alive), Dende until Guru, Nail (temporary) | Guru power-up, 1-star ball, Zarbon, Vegeta joins (N-01 to N-04). |
| 9 | Earth - Goku's training 2 / 超重力修行 (spaceship) | Spaceship | Goku | Second gravity block; its time affects the Ginyu arrival (N-06, N-09). |
| 10 | Planet Namek - Showdown! The Ginyu Special Squadron! / 対決！特戦隊！ | Namek valleys, landing spots | Gohan, Krillin, survivors, Vegeta; Goku arrives by timing | Recoome, Guldo, Jeice, Burter, Ginyu. Frieza in the north cave (N-07 to N-12). |
| 11 | Planet Namek - Nail vs. Frieza / ネイル対フリーザ | Guru's area | Nail alone | Nail must hold out. Then the Porunga wish revives Piccolo (N-13, N-14). |
| 12 | Planet Namek - Freeza the Planet Destroyer / 激神フリーザ！ | Namek | Piccolo (revived; may fuse with Nail), Gohan, Vegeta | Frieza 2nd form (F-01, F-02). |
| 13 | Planet Namek - Battle Freeza / 決戦フリーザ！ | Namek | Piccolo, Gohan, Vegeta, Dende (if not fused), Goku from the tank | Maima and Tsumuri choice (F-03), Frieza 3rd form (F-05), final form (F-06). |
| 14 | Super Vegeta / 超ベジータ (hidden, after credits) | Namek | Goku (Super Saiyan) vs Vegeta | Only if Goku went Super Saiyan and Vegeta is alive (E-02). |

Save points for test play:

| ID | Point in the story | Why |
|---|---|---|
| SP-01 | New game, Kame House | Baseline. |
| SP-02 | Raditz forced fight (Goku and Piccolo at Level 4, Gohan card in hand) | S-03. |
| SP-03 | King Kai, first gravity trial starts | The return clock starts here (S-05). |
| SP-04 | Earth "Vegeta attacks" start: Piccolo below Level 9, Nappa not yet met | Recruit window (S-08 to S-10). Most important point. |
| SP-05 | Just before the Nappa fight | Goku-arrival phases (S-07). |
| SP-06 | After Nappa, before the Vegeta duel | S-15. |
| SP-07 | Namek arrival (Frieza appears) | Start of map 5. |
| SP-08 | Guru's house, before stepping onto the pots | N-01. |
| SP-09 | Zarbon fight starts (test with and without the Vegeta card) | N-04. |
| SP-10 | Ginyu map start | N-07. |
| SP-11 | Nail vs Frieza start | N-13 (test the turn count). |
| SP-12 | Seven Dragon Balls in inventory, just before Porunga is summoned | N-14. |
| SP-13 | Frieza 2nd form has run off; Maima and Tsumuri are reachable | F-03 (test with and without Piccolo in the party). |
| SP-14 | Final Frieza battle starts, Goku still in the tank | F-06. |
| SP-15 | After Frieza dies, at the credits | E-02. |

---

## 1. Saiyan arc on Earth (maps 1 to 4)

### S-01 Gohan card from Mr. Popo's house
- Where/when: map 1, the house north of Kame House in the mountains.
- Trigger: defeat the Saibamen that attack Mr. Popo's house.
- Result: the Gohan card. Raditz can only be damaged with it.
- Certainty: [B] RPGC walkthrough; JAWIKI (Raditz section); LUC card table.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://lucifer.s14.xrea.com/hobby/supersaiya/card/index.htm

### S-02 Fake Raditz at the cave exit
- Where/when: map 1, exit of the first cave.
- Trigger: go through the cave. The copy-Raditz waits at the exit.
- Result: boss fight (RPGC lists HP 150, BP 800). You must beat it to continue normally.
- Certainty: [A] JAWIKI; LUC enemy page; FANDOM bosses list.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/enemystates/raditzfake.htm ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu

### S-03 Forced Raditz fight at Level 4
- Where/when: map 1, after S-02, in the cave.
- Trigger: Goku and Piccolo both at Level 4, and the Gohan card in inventory. Raditz then attacks you.
- Side effect: with two characters at Level 4, random encounters stop on that map (JAWIKI footnote; LUC says enemies flee).
- Certainty: [A] LUC map 1st; JAWIKI.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/1st/index.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### S-04 The Raditz fight: Gohan card, grab, Goku's sacrifice
- Where/when: the real Raditz, end of map 1.
- Trigger: use the Gohan card before anything else. Gohan attacks, and a new option appears where Goku grabs Raditz.
- Result: Piccolo's Demon Ray can finish both at once. The walkthrough also describes healing Goku at 1 to 5 HP with Kami cards until he reaches Level 7 (see G-01). LUC says Goku's self-sacrifice is needed and he dies anyway.
- Certainty: [B] LUC says Goku dies in this fight (the afterlife training follows). RPGC describes healing him through the fight. Test whether he survives the grab.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/enemystates/raditz.htm

### S-05 Goku's afterlife training at King Kai's (sets Goku's return)
- Where/when: King Kai's planet, after Raditz (Goku is dead). Map 3.
- Trigger: clear the 10x gravity trial within its turn limit (no follow-up attacks, no counters). Failing gives a small XP reward and a retry; quitting gives nothing. RPGC says you clear three trials in a row, then fight King Kai (BP 4,000, weak).
- Result: Goku returns to the world. The time this takes sets his arrival (S-07). Failures delay the return.
- Certainty: [A] training structure (JAWIKI; RPGC). [B] failures delay the return (LUC map practice2 only).
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/practice/index2.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### S-06 Gohan's practice fight against Piccolo
- Where/when: map 2, Earth, after Raditz.
- Trigger: train with Piccolo and accept the practice fight.
- Result: a practice battle. Losing is not a game over (a spring heals).
- Certainty: [B] LUC map practice; RPGC.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/practice/index.htm ; http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml

### S-07 Goku's arrival in the Nappa chapter (timing phases)
- Where/when: map 4, "Vegeta attacks".
- Trigger: the time spent in S-05. Four arrival phases: at the start of the map; during the first Saibaman fight; during the Nappa fight; after Nappa.
- Result: the earlier he arrives, the easier Nappa is and the fewer allies die. The latest case has Nappa and Vegeta back to back until Goku shows up.
- FANDOM also says Goku can return before Nappa if Piccolo kills Raditz with an attack other than Makankosappo and Goku makes no mistakes at King Kai's. Single source, [C].
- Certainty: [A] LUC map 2nd; JAWIKI (Vegeta and Goku sections). The actual clock (frames, steps, or actions) is not stated; measure it in tests.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/2nd/index.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### S-08 Recruit window for Krillin, Yamcha, Tien and Chiaotzu
- Where/when: map 4, Earth, after Gohan and Piccolo start moving.
- Locations: Krillin south of the start; Tien and Chiaotzu far south, by the ocean and a cliff; Yamcha far to the left of where the Saiyans stand (RPGC).
- Trigger: walk into each of them to recruit.
- Recruit gifts: Yamcha gives the Puar card; Tien and Chiaotzu give the Flight card and the Grandpa Gohan card.
- Certainty: [A] locations (RPGC); gifts (LUC map 2nd); deadlines (JAWIKI).
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/2nd/index.htm

### S-09 Deadline 1: Piccolo reaches Level 9
- Trigger: Piccolo reaches Level 9 while any of the four recruits is still unrecruited.
- Result: that fighter is dead for good and cannot be recruited. Random encounters also stop until the Nappa fight is over. This is the main way to lose the recruits without noticing.
- Certainty: [A] JAWIKI footnote; LUC map 2nd (enemies flee at Level 9).
- Sources: https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/2nd/index.htm

### S-10 Deadline 2: walking into Nappa
- Trigger: walk into Nappa on the field while a recruit is still unrecruited.
- Result: that fighter challenges Nappa alone and dies, and the dialogue changes.
- Certainty: [B] JAWIKI (Nappa section and footnote). Single detailed source.
- Sources: https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### S-11 Yamcha, Tien and Chiaotzu reach Namek alive (an example)
- Trigger, in order: recruit all of them before S-09 and S-10 (SP-04); keep Piccolo below Level 9 until then; finish the King Kai trial fast enough that Goku arrives at the start of map 4 (S-07, fastest phase); then get through the Nappa fight without losing any of them.
- Result: they survive Nappa, accompany Gohan to Namek, and receive Guru's power-up if they reach him.
- Fallback: if they die, a later revival with Piccolo may bring them back (see N-14). JAWIKI says Z-fighters who died before the Guru meeting and revive together with Piccolo change school. This is inference, so it is marked [C].
- Certainty: [B] the survival path (FANDOM trivia; LUC; JAWIKI). [C] revival with Piccolo.
- Sources: https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/2nd/index.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### S-12 Piccolo's scripted death in the Nappa fight
- Trigger: Piccolo is still alive when Nappa is defeated.
- Result: an event has Piccolo shield Gohan and die. Piccolo's death before the Raditz fight is a game over.
- Certainty: [A] JAWIKI (Piccolo and Nappa sections); RPGC characters page.
- Sources: https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://shrines.rpgclassics.com/snes/dbzlotss/characters.shtml

### S-13 The Nappa fight itself
- Trigger: any time in map 4 after the Goku-arrival phase.
- Result: Nappa attacks twice a turn. He is very strong without Goku and easy with him. Killing him sends Goku and Vegeta away.
- Certainty: [A] RPGC; JAWIKI; LUC map 2nd.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/2nd/index.htm

### S-14 Moon/Tail card (Gohan's Great Ape)
- Where/when: a reward from the Nappa fight.
- Trigger: win against Nappa.
- Result: the Moon/Tail card turns Gohan into a Great Ape in battle (BP roughly ten times normal, JAWIKI). Keep it past the Vegeta duel and it disappears (LUC).
- Certainty: [A] reward (RPGC; LUC); [B] disappearing rule (LUC only).
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/states/gohan_m.htm

### S-15 Vegeta duel (Goku alone)
- Where/when: after Nappa, map 4.
- Trigger: Goku and Vegeta fly off together. Keep everyone else out of the duel. The walkthrough uses the X-button prompt and answers yes.
- Result: Vegeta can turn into a Great Ape from a Power Ball, once. Reverting him uses Krillin's Ki-en-zan or the Yajirobe card (cuts the tail). After the duel you receive the Vegeta card.
- Vegeta also refuses or skips turns at times (FANDOM, LUC, TAS). Treat as noise.
- Certainty: [A] duel and card reward; [B] Power Ball and tail rules.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/2nd/index.htm

### S-16 Gohan's death is a game over before Ginyu
- Trigger: Gohan dies at any point from the Nappa fight through the Ginyu fights.
- Result: game over. The LUC Gohan page says any Gohan death is a game over.
- Gohan can also be stopped by "fear" messages and erratic multi-attacks in battle (JAWIKI).
- Certainty: [A] JAWIKI; LUC states/gohan.htm.
- Sources: https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://lucifer.s14.xrea.com/hobby/supersaiya/states/gohan.htm

---

## 2. Travel to Namek (maps 5 to 7, including the spaceship)

### TR-01 Dragon Radar at the start of map 5
- Trigger: the Dragon Radar was not picked up on map 1. In that case it is given at the start of map 5.
- Certainty: [B] LUC map 3rd.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/3rd/index.htm

### TR-02 Dende's rescue
- Where/when: map 5, Frieza's town.
- Trigger: rescue Dende from Frieza's men (the first objective of the map).
- Result: Dende joins. He cannot fight. He can be levelled only up to 3 (FANDOM trivia; the repo's character table also stops at 3). At Level 3 he fully heals one ally per turn. The later fusion choice (F-03) depends on him being in the party.
- Certainty: [A] LUC map 3rd and states/den.htm; JAWIKI.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/3rd/index.htm ; http://lucifer.s14.xrea.com/hobby/supersaiya/states/den.htm

### TR-03 Cargo, Dende's brother
- Where/when: map 5, in one of the village houses.
- Trigger: check the pots in the houses until you find him (RPGC).
- Result: Cargo joins. He does not fight or level up. He is crushed by Frieza later (JAWIKI).
- Side effect: contacting Dende, Cargo, or Maima and Tsumuri makes Cui disappear from the map (LUC map 3rd).
- Certainty: [A] RPGC; LUC; JAWIKI.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/3rd/index.htm

### TR-04 Maima and Tsumuri and the 2-star ball
- Where/when: map 5, north Namek. Maima BP 3,300 and Tsumuri BP 3,200, both fixed at Level 15 (JAWIKI).
- Trigger and result, normal case: with Dende or Cargo in the party, talking to them gives the 2-star ball, and they join temporarily (RPGC).
- Trigger, bad case: with neither in the party, talking to them starts a fight with both. Losing either is a game over, so flee.
- Trap: if you finish the Dende rescue after that fight, both are left badly hurt. One healing helper card saves them and they join. Without one, both die. There is no game over, but the late Piccolo-Dende fusion (F-03) is lost.
- Both leave the party after Dodoria (TR-06) and return late in the game (F-03).
- Certainty: [A] fight rule (JAWIKI; LUC enemy pages for Maima and Tsumuri). [B] near-death rule (JAWIKI only).
- Sources: https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://lucifer.s14.xrea.com/hobby/supersaiya/enemystates/mai.htm ; http://lucifer.s14.xrea.com/hobby/supersaiya/enemystates/tsum.htm ; http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml

### TR-05 The 6-star ball in the cave and the Cui fight
- Where/when: map 5, the southeast cave. Build BP up first (RPGC suggests about 9,000 to 10,000).
- Trigger: take the 6-star ball, then leave the cave. The Cui fight starts at the exit.
- Result: Cui (RPGC lists BP 18,000). FANDOM says Cui can also be fought before Frieza, or after the cave ball.
- Certainty: [B] LUC map 3rd; FANDOM; RPGC. The "before Frieza" option is FANDOM only.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/3rd/index.htm ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu

### TR-06 The "big ki" message and Dodoria at the ship
- Trigger: after the map 5 objectives, a message says a strong ki is approaching. Walk to the ship (the start point) and Dodoria attacks.
- Result: Dodoria fight (RPGC lists BP 22,000). Afterwards Bulma appears, the group flies off, and Maima and Tsumuri leave the party.
- Certainty: [A] LUC map 3rd; RPGC; LUC tsum/mai pages.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/3rd/index.htm ; http://shrines.rpgclassics.com/snes/dbzlotss/cheats.shtml

### TR-07 Scouter before Dodoria (stat carry-over)
- Trigger: use a Scouter before the Dodoria fight. Later, Goku in the spaceship and Piccolo on King Kai's planet pick up stats from earlier fighters. The sources name Gravitron, Nappa and Raditz.
- Certainty: [C] two tips pages (RPGC cheats; CH). Mechanism unknown.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/cheats.shtml ; https://criticalhits.com.br/dicas/dragon-ball-z-super-saiya-densetsu-cheats-codigos-dicas-e-desbloqueaveis/

### TR-08 Spaceship gravity trials, block 1 (chapter labelled "Earth - Goku's training")
- Where/when: map 6, the spaceship gravity room. Goku alone.
- Trigger: go upstairs and choose the first option (the walkthrough notes the prompt exists only in the Japanese ROM; see V-02). Pick a gravity from 10x upward (the walkthrough suggests starting at 20x). Clear each trial to raise the gravity. Training ends when Goku decides to rest. The map's objective is five clears.
- Result: clearing gives +2 levels directly (JAWIKI, device-specific). The gravity you train at counts toward XP, and each attempt is logged. Healing and waiting delay Goku's arrival on Namek (LUC map gokuuship).
- Certainty: [A] trials exist; [B] +2 levels; [C] arrival-delay effect.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/gokuuship/index.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### TR-09 Piccolo's afterlife training at King Kai's (map 7)
- Where/when: King Kai's planet, after TR-08. Piccolo is dead and trains alone.
- Trigger: the story switches to Piccolo. Clear the training.
- Result: the training opponents are Nappa and Raditz, brought back from hell by King Kai for this purpose.
- Piccolo's revival is not here. It comes with the Porunga wish (N-14).
- Certainty: [B] RPGC; LUC map picolokaiousei; JAWIKI.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/picolokaiousei/index.htm ; http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml

---

## 3. Namek arc (maps 8 to 11)

### N-01 Guru's house: power-up and the 1-star ball
- Where/when: map 8, the field past the cave with the two Namekians and Nail. Guru's house sits on a hill.
- Trigger: enter the house, then step into the middle of the pots. Guru takes you upstairs.
- Result: Guru raises the BP of everyone in the party, gives you the 1-star ball, and Dende and Nail leave the party. Z-fighters' school changes to planet (JAWIKI).
- Certainty: [A] RPGC; JAWIKI; LUC map 4th.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/4th/index.htm

### N-02 Nail (temporary) and the fake Nail
- Where/when: map 8. The cave north of the hill has two Namekians fighting. Joining them makes Nail a temporary ally. Nail has BP 42,000 and no levels.
- Fake Nail: a copy-man in the cave leading to Guru. BP 31,500, slightly above Zarbon's transformed form. Ignoring it is fine.
- Certainty: [A] Nail's role and BP; [B] fake Nail.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/enemystates/fakenail.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### N-03 Zarbon fight and Vegeta joins permanently
- Where/when: map 8, after Guru. Leave by the cave's outer exit.
- Trigger: the exit starts the Zarbon fight.
- Gotcha: if you hold the Vegeta card when the fight starts, the card vanishes.
- Result: after the fight, Vegeta joins permanently. Zarbon transforms (BP 23,000, then 30,000).
- Certainty: [A] LUC map 4th; LUC states/vegeta.htm; RPGC; JAWIKI.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/4th/index.htm ; http://lucifer.s14.xrea.com/hobby/supersaiya/states/vegeta.htm

### N-04 Fake Frieza and the real Frieza at Guru's house
- Where/when: map 8 (inferred from the Guru-house sequence; the source gives no location for the copy).
- Trigger: lower the fake's HP and keep the fight going. It retreats and reports to the real Frieza, who executes it. The real Frieza then waits outside Guru's house and attacks as a forced encounter.
- Certainty: [B] JAWIKI only. The LUC page confirms the copy is weaker than the Ginyu Force.
- Sources: https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://lucifer.s14.xrea.com/hobby/supersaiya/enemystates/fakefreeza.htm

### N-05 Spaceship gravity trials, block 2 (chapter labelled "Earth - Goku's training")
- Where/when: map 9, after Zarbon. Goku alone in the spaceship.
- Trigger and rules: as TR-08.
- Result: the time spent here decides which phase of the Ginyu chapter Goku arrives in (N-08).
- Certainty: [A] trials exist; [B] arrival effect.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/gokuuship/index.htm

### N-06 Ginyu chapter, first fight: Recoome and Guldo
- Where/when: map 10, Namek valley where the Dragon Balls are.
- Trigger: the map starts with a forced fight against Recoome and Guldo (BP 65,000 and 13,500 in RPGC).
- Result: Ginyu steals the Dragon Balls. You must recover them later.
- Certainty: [A] LUC map 5th; RPGC.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/5th/index.htm ; http://shrines.rpgclassics.com/snes/dbzlotss/bosses.shtml

### N-07 Ginyu chapter: Jeice and Burter
- Trigger: land on a spot outside the red boundary without fighting them.
- Result: forced fight. When you beat one, the other flees to fetch Ginyu (RPGC; JAWIKI).
- Certainty: [A] LUC map 5th; RPGC; JAWIKI.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/5th/index.htm ; http://shrines.rpgclassics.com/snes/dbzlotss/bosses.shtml

### N-08 Goku's arrival phase in the Ginyu chapter
- Trigger: the time spent in N-05. Three phases: Goku arrives during the Recoome and Guldo fight; during the Jeice and Burter fight; or during the Ginyu fight.
- Result: if he arrives late, Gohan, Krillin, any survivors and Vegeta have to hold the Ginyu crew without him (FANDOM).
- Certainty: [A] phases (LUC map 5th; JAWIKI note); [B] the no-Goku fight (FANDOM).
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/5th/index.htm ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu

### N-09 Goku against Ginyu (forced duel and body swap)
- Where/when: map 10. Goku arrives with the phase from N-08.
- Trigger: Ginyu is not yet defeated and you step onto a landing spot inside the blue boundary. This starts a forced one-on-one duel with Goku. Helper cards cannot be used in it.
- Result: Ginyu switches bodies with Goku at low HP. If you beat Ginyu before the swap, the swap event still plays and ends the fight. Goku's death here is not a game over (RPGC).
- Namekian Frog: the sources disagree. JAWIKI says a Frog obtained on Namek blocks swaps onto other allies and lets you force the win. LUC says the Frog is a swap target and can be traded to Yajirobe for the Yajirobe card. RPGC only describes the Oolong trade that makes the Frog.
- Certainty: [A] duel and swap; [C] Frog effect.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/5th/index.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/card/index.htm

### N-10 Captain Ginyu (playable Ginyu)
- Trigger: during the duel, let Ginyu's swap land on one of your Z-fighters, then give that fighter the order to switch so Ginyu takes command.
- Result: you control Ginyu. He can swap with any character (including Frieza or Super Vegeta) until the fight ends (FANDOM).
- Certainty: [B] existence (two sources); [C] exact steps.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/cheats.shtml ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu

### N-11 Frieza in the north cave (end of map 10)
- Where/when: the cave north of Goku's landing area, after the Ginyu fights.
- Trigger: enter the cave.
- Result: Frieza (1st form) is inside and is easy to beat. Collect the Dragon Balls there and bring them back.
- Certainty: [B] RPGC; LUC map 5th lists "retrieve the Dragon Balls" as an objective.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/5th/index.htm

### N-12 Dragon Ball locations (partial)
- Known: 2-star from Maima and Tsumuri (TR-04); 6-star in the map 5 cave (TR-05); 1-star from Guru (N-01); Ginyu's stolen balls (N-06); a ball on a hill and a ball in a north cave (RPGC, map 8 area); the Frieza cave ball (N-11).
- Not documented: where the rest are, and whether any sit on Earth. Porunga needs seven.
- Certainty: [B] for the locations listed; gap for the rest.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/3rd/index.htm

### N-13 Nail vs Frieza
- Where/when: map 11, Guru's area. A scripted duel: you control Nail, and Frieza attacks.
- Rule: Nail cannot win. He must stay alive for a set number of turns. LUC's map page says surviving through turn 6 (dying on turn 7 or later) earns the reward. LUC's Nail and Moon/Tail pages say 5 turns. RPGC says the story needs Nail to lose eventually.
- Reward: Porunga card or Moon/Tail card (LUC map page). FANDOM says the same: keep Frieza busy long enough and Porunga grants a wish or a card.
- Helpful: Kami, Baba and Grandpa Gohan cards, as the LUC map suggests.
- Certainty: [B] reward; [C] turn count (5 vs 6).
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/freezavsnail/index.htm ; http://lucifer.s14.xrea.com/hobby/supersaiya/states/nai.htm ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu

### N-14 Porunga summon and Piccolo's revival
- Trigger: after N-13, with all seven Dragon Balls in inventory. Dende warns that Frieza is coming, and the group summons Porunga.
- Result: the first wish brings Piccolo back to life and to Namek (RPGC; GB). If Nail hit Frieza during N-13, an extra question offers the Porunga card instead of a second wish (RPGC).
- Other wishes: Porunga grants wishes, not Shenron. No Earth wish is documented in any source.
- Group revival: JAWIKI implies revivals can happen with Piccolo for other dead Z-fighters (not Gohan). Unverified.
- Missable: Piccolo returns only through this one-time revival. Without all seven balls here, he never joins (inference from JAWIKI's "one revival event only" rule, [C]).
- Certainty: [A] Piccolo's return with the wish; [B] extra question; [C] group revival and the missable reading.
- Checked in the code and in a replay (`secret-piccolo-returns`): the wish scene itself does not put Piccolo in the party. The chapter set-up of map 22 saves the ids of the characters 3 to 7 (Krillin, Tien, Yamcha, Chiaotzu, Vegeta) that are not in the party into a list in the image (`$7E:29A0`), and the story step `$16` (the caption after Frieza's first form leaves, scene `$42` then `$38`) runs scene `$38`: Piccolo says he has arrived and the party becomes Piccolo followed by that list (the old party is stored in the roster, not kept). So he returns after the first forced Frieza fight on map 22, not at the wish. In a real run the list holds whoever of those five was away at that time; the replay uses Tien, Yamcha and Chiaotzu.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; https://www.giantbomb.com/games/3030-22810/ ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### N-15 The Porunga card (battle item)
- Effects (each usable once, value drops 50 points per use): all cards in hand become Z-star and Z-number with the 必 school; one character gains a level; all enemies lose half their HP.
- Price 150 points (RPGC items list). RPGC says you get it if Nail hits Frieza.
- Certainty: [A] effects (LUC card table; RPGC items agree on three wishes).
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/card/index.htm ; http://shrines.rpgclassics.com/snes/dbzlotss/items.shtml

---

## 4. Frieza (maps 12 and 13)

### F-01 Piccolo fuses with Nail (optional)
- Where/when: map 12, "Freeza the Planet Destroyer", after the revival in N-14. Nail is lying on the ground after Frieza's attack.
- Trigger: find Nail and accept the fusion. LUC lists it as a map objective. JAWIKI says it can be skipped.
- Result: Piccolo gets a large power boost. GB says Level 48 after the fusion. FANDOM says it matches what grinding to Level 47 would give. Skipping means joining at the first-form fight without the boost (JAWIKI).
- Certainty: [A] fusion exists and is optional; [C] the exact level.
- Sources: https://www.giantbomb.com/games/3030-22810/ ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://lucifer.s14.xrea.com/hobby/supersaiya/map/6th/index.htm

### F-02 Frieza 2nd form
- Where/when: map 12. Frieza first appears in guerrilla fashion, then fights on a small island.
- Trigger: push Frieza until he transforms.
- Result: 2nd-form battle (RPGC: HP 520, BP 1,000,000). After a round or two Piccolo joins with over a million BP. Frieza runs off once his HP is low.
- JAWIKI says the 2nd form is roughly as strong as a Nail-fused Piccolo at max level. LUC says the fight is hard without the Dende fusion.
- Certainty: [A] RPGC; JAWIKI; LUC freeza2 page.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/bosses.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/enemystates/freeza2.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### F-03 Maima and Tsumuri: the Piccolo-Dende fusion choice
- Where/when: the area where you fought Ginyu's crew, after F-02 (maps 12 and 13).
- Trigger: talk to Maima and Tsumuri while Piccolo is in the party. JAWIKI says the choice appears automatically when Piccolo is present.
- Without Piccolo: they hand over a strong helper card instead, and there is no fusion.
- Choice yes: Piccolo fuses with Dende. Piccolo's BP becomes about 1.9 million, his school becomes Super (超), and he is forced to Level 48. Dende leaves, so his healing is gone (GB; JAWIKI).
- Choice no: Dende stays and keeps healing. He is killed later by Frieza (F-04).
- Who fuses: RPGC and JAWIKI name Dende only. FANDOM says Dende and both Namekians.
- Certainty: [A] the choice and its effects; [B] who fuses.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; https://www.giantbomb.com/games/3030-22810/ ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu
- Checked in the code and in a replay (`secret-piccolo-dende-fusion`): the trigger is on map 24 (also in the object list of map 23), not on the Ginyu squad's maps. Two Namekian objects stand side by side at x 1656 and 1672, y 1152 (the loader's list entries `$2F` and `$30`). Talking to either one starts scene `$3C` only while story flag 2 is clear; with it set they say a short line (the same message for both). The scene first checks whether Piccolo's record is in the party and keeps that as flag 1. With Piccolo in the party: a white flash, the two Namekians vanish (flags `0x10` and `0x11` hide them), a voice says "Fuse with me, too! I want to help fight Frieza!", Piccolo's power-up animation, "I'm more powerful than before!", and Piccolo's level goes up (observed: 40 to 48, which fits the "forced to Level 48" of the guides; the rise is 4, or 2 when one of the pair was already gone, plus more later in the scene). Dende's record is removed from the party when Krillin (flag 3) is absent; with Krillin present the scene ends with one more line and Dende stays. Vegeta (flag 7) in the party changes the last lines. Without Piccolo the scene only plays lines and sets flag 2 (no card is given by the script; the helper card of the guides was not seen). No yes/no choice was found: the fusion is automatic once Piccolo is there.
- Sources for the check: the fandom wiki page of Maima (Piccolo can fuse with Maima and Tsumuri, Dende can join the fusion, near the end of the game) https://dragonball.fandom.com/wiki/Maima ; the replay and the code of scene `$3C` (`data/scenes.md`).

### F-04 Dende's death in the no-fusion route (possible SSJ trigger)
- Trigger: choose "no" in F-03 and reach the final battle.
- Result: Frieza kills Dende after his final transformation, as in the original (JAWIKI).
- Open question: whether Dende's death counts as the ally death that triggers Goku's Super Saiyan form (F-06). Untested, so test it.
- Certainty: [A] Dende dies; [C] SSJ link.
- Sources: https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://lucifer.s14.xrea.com/hobby/supersaiya/states/den.htm

### F-05 Frieza 3rd form: random encounters, then the main battle
- Where/when: maps 12 and 13. After the 2nd form runs off, the 3rd form appears as random encounters, roughly every third or fourth fight.
- Trigger: four encounters, per LUC's map page, start the main battle. JAWIKI says the 3rd form can only be beaten in the main battle.
- Result: the 3rd form has BP 1,555,555 (RPGC). When it dies it transforms into the final form. Piccolo's Demon Ray works well on the 3rd form (RPGC).
- Certainty: [A] encounter pattern (LUC map last; RPGC; JAWIKI). The count of four is LUC only.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/last/index.htm ; http://lucifer.s14.xrea.com/hobby/supersaiya/enemystates/freeza3.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### F-06 Goku's Super Saiyan trigger (final battle)
- Where/when: map 13, the final Frieza form. Goku comes out of the healing tank partway through the fight and must be alive.
- Trigger: any ally other than Goku dies during this battle. Gohan's death is a game over. Vegeta must be alive at the end for E-02.
- Victim list, which varies by source: Chiaotzu (RPGC walkthrough); Krillin or Chiaotzu (GB); Krillin, Yamcha, Tien or Chiaotzu (RPGC cheats; CH); "anyone" (LUC and JAWIKI). Test each candidate.
- Result: Goku is forced to Level 48 and BP 4,999,999 (fixed). Attack and defence jump. Kaioken and Genki Dama are disabled. Charged attacks are cancelled (JAWIKI footnote; LUC sgoku page).
- Certainty: [A] trigger type and result; [B] victim list.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/enemystates/freeza3.htm ; http://lucifer.s14.xrea.com/hobby/supersaiya/states/sgoku.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://shrines.rpgclassics.com/snes/dbzlotss/cheats.shtml ; https://www.giantbomb.com/games/3030-22810/ ; https://criticalhits.com.br/dicas/dragon-ball-z-super-saiya-densetsu-cheats-codigos-dicas-e-desbloqueaveis/

### F-07 Sachioro card on Super Saiyan Goku (BP number)
- Where/when: final battle, after F-06.
- Trigger: use a Sachioro card on Goku. In this build, Sachioro is the Guru card (最長老), which gives +50% BP in battle.
- Identification: the card names are a match by name and effect (LUC's Guru card gives +50%, the same as RPGC's Sachioro). Inference, [B].
- Result: 4,999,999 becomes 7,499,998 (RPGC walkthrough). RPGC cheats and CH say 7,999,998, which does not match +50%. Treat 7,499,998 as correct unless the test shows otherwise.
- Shops: the walkthrough says Goku passes shops where Chi-Chi and Sachioro cards can be bought before the final battle. The shop is not named (gap).
- Certainty: [B] the +50% effect; [C] the 7,999,998 figure.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; http://shrines.rpgclassics.com/snes/dbzlotss/cheats.shtml ; http://lucifer.s14.xrea.com/hobby/supersaiya/card/index.htm

### F-08 Victory over the final form
- Trigger: kill the final Frieza.
- Result: credits. Then E-02 is checked.
- Certainty: [A].
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml

---

## 5. Endings (map 14 and after the credits)

### E-01 Normal ending
- Trigger: Frieza's final form is beaten, and the Super Vegeta conditions in E-02 are not met.
- Result: credits and the standard ending.
- Certainty: [A].
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml

### E-02 Super Vegeta (hidden boss, the "best" ending)
- Where/when: after the credits.
- Trigger: both must hold. (a) Goku became Super Saiyan through F-06. (b) Vegeta is alive at the end.
- Result: a fight with Super Vegeta. He wears the arrival outfit and has red eyes (JAWIKI). RPGC lists HP 1,800 and BP 6,000,000. Winning gives the extra ending (JAWIKI calls it the true ending; TAS calls it "best ending").
- Tips (RPGC): heal Goku, use Sachioro on him, and use Chi-Chi cards to turn regular cards into Ki cards. Use Goku's and Piccolo's strongest Ki attacks. LUC says the fight is hard if your Guru cards are used up.
- Certainty: [A] the conditions and the fight (LUC ending page; JAWIKI; RPGC; TAS runs).
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/ending/index.htm ; http://lucifer.s14.xrea.com/hobby/supersaiya/enemystates/svegeta.htm ; https://tasvideos.org/4486S (notes: "best ending" means meeting SSJ Vegeta) ; https://tasvideos.org/5962S ; https://tasvideos.org/3707M ; http://shrines.rpgclassics.com/snes/dbzlotss/bosses.shtml

### E-03 Vegeta must survive to the end
- Vegeta joins after Zarbon (N-03). The sources say the hidden boss needs Vegeta not to have died (LUC: "if Vegeta is not dead"; JAWIKI: "if Vegeta survived"). They do not say whether a death earlier in the game counts, so protect him from Ginyu through the end.
- Certainty: [A] LUC ending page; JAWIKI; RPGC.
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/map/ending/index.htm ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### E-04 Game-over triggers (summary)
- Gohan dies from the Nappa fight to the Ginyu fight (S-16).
- Piccolo dies before the Raditz fight (S-12).
- Losing to either Maima or Tsumuri in the forced fight (TR-04).
- Any death of a story-critical character (JAWIKI general rule). The list of which characters count is not given beyond the examples above.
- Certainty: [A] for the listed items; [B] for the general rule.
- Sources: https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; http://lucifer.s14.xrea.com/hobby/supersaiya/states/gohan.htm

---

## 6. Cards and items

Card names are from the English patch where RPGC or LUC uses them. The Japanese name is in brackets.
Map and battle columns show where the item can be used by default (LUC card table). The trick in I-03 can
bypass these limits.

| Card (JP) | Effect | Map / battle | Price (RPGC) | Notes |
|---|---|---|---|---|
| Bulma (ブルマ) | +25 HP, one character | both | 3 | |
| Kame (亀仙人) | +50 HP, one character | both | 6 | |
| Kami (神様) | Full HP, one character | both | 10 | Saiyan Power trick (G-01). |
| Puar (プーアル) | +10 KI, one character | both | 4 | Yamcha gives one on recruit (S-08). |
| Mr. Popo (ミスターポポ) | Full KI, one character | both | 10 | |
| Korin (カリン様) | Full HP and KI, one character | both | 20 | |
| Shenlong (神龍) | Full HP and KI, whole party | both | 60 | A heal card, not a wish. |
| Enma (エンマ様) | One card's stars become Z | both | 15 | |
| Baba (占いババ) | One card's number becomes Z | both | 15 | |
| Chi-Chi (チチ) | One card's school becomes 必 (Ki cards, per RPGC) | both | 12 | RPGC suggests it for the SSJ Vegeta fight (E-02). |
| Sea turtle (ウミガメ) | Stars and numbers swapped on all cards (LUC). RPGC says it equalizes attack and defence. | both | 6 | Build conflict (V-05). |
| Oolong (ウーロン) | Replaces the whole hand | both | 4 | |
| Goku (悟空) | One card to Z-star and Z-number, school 気 | battle only | 40 | |
| Lunch (ランチ) | One attack hits all enemies this turn; may fail | battle only | 10 | |
| King Kai (界王様) | +25% BP, one character, this battle | battle only | 20 | Battle-only BP; see G-03 for the disputed permanent claim. |
| Guru (最長老, "Sachioro") | +50% BP, one character, this battle | battle only | 40 | See F-07. |
| Grandpa Gohan (じいちゃん) | Stops one enemy | battle only | 20 | Recruit gift from Tien and Chiaotzu. |
| Piccolo (ピッコロ) | One enemy card becomes 1 and school 神 | battle only | 20 | LUC: can be cancelled after you see the enemy's cards (I-04). |
| Yajirobe (ヤジロベー) | Damages one enemy; cuts Great Ape Vegeta's tail | battle only | 20 | Given to you when you hand Yajirobe a Namekian Frog (LUC; FANDOM). |
| Scouter A | Shows enemy HP, KI, BP and cards for 2 or 3 uses | map | 10 | LUC: breaks on meeting BP 29,999 or more. RPGC: shows BP up to 23,000 over three battles. |
| Scouter B | Same, 5 uses | map | 20 | LUC: breaks on meeting BP 299,999 or more. RPGC: shows BP up to 199,999 over five battles. |
| Bubbles (バブルス) | No random encounters for a while | map | 10 | |
| Flight (舞空術) / "Escape" in RPGC | Map: back to last healing point. Dungeon: back to the entrance | map | 15 | |
| Vegeta (ベジータ) | Auto-controlled Vegeta helps this battle | battle only | 20 | Not the same as the joining Vegeta (LUC). |
| Moon/Tail (月と尻尾) | Gohan turns into Great Ape | battle only | 80 | See S-14 and N-13. |
| Porunga (ポルンガ) | Three one-time effects (N-15) | battle only | 150 | |
| Dragon Radar (ドラゴンレーダー) | Direction and rough distance to Dragon Balls | map | not sold | TR-01. |
| Gohan (ゴハン) | Needed to damage Raditz | battle only | not sold | S-01 and S-03. |
| Namekian Frog (カエル) | Target of Ginyu's body swap (LUC); handed to Yajirobe for the Yajirobe card | battle only | from Oolong (see below) | N-09 (sources disagree). |
| 1 to 7 star Dragon Balls (シンチュウ) | Event cards (store the balls) | not usable | not sold | N-12, N-14. |

Shops and trades:
- Fortune-teller Baba buys and sells item cards for zeni in her house on Earth. After the move to Namek, a Namekian salesman replaces her store (FANDOM, JAWIKI).
- Training center at Kame House: Ki training and paid battles (see X-01).
- Oolong trades a Lunch card for the Namekian Frog (RPGC items; FANDOM). Where Oolong is found is not given (gap).
- Yajirobe trades the Frog for the Yajirobe card (LUC card table; FANDOM). Where Yajirobe is found is not given (gap).
- The Japanese wiki says helper cards are bought and sold at a card exchange for credits, with a 24-card carry limit. Dragon Balls count against that limit.

Certainty: [A] the card list (LUC and RPGC agree on nearly all entries); [B] price and availability; [C] the Frog and Turtle conflicts.
Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/card/index.htm ; http://shrines.rpgclassics.com/snes/dbzlotss/items.shtml ; https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu

### I-01 Battle turn order
- Rule (LUC): the card with more stars acts first, then the higher kanji number. Ties are not resolved.
- Rule (FANDOM): attack power first, then defence, then BP. The two rules disagree.
- Certainty: [C].
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/others/index.htm ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu

### I-02 Battle structure (for save-state and test design)
- One card per active character. A battle can start with any number of characters (JAWIKI).
- The hand refills to five at the end of the turn.
- A character at 0 HP is dead until a one-time revival event.
- Level cap is 48 for most characters. Goku, Piccolo and Vegeta stop at 47 by XP. Goku and Piccolo reach 48 through events (JAWIKI footnote, FANDOM).
- School changes: planet school after the Guru meeting; King Kai school (界) for Z-fighters who died before Guru and came back with Piccolo; Super school (超) after fusion or SSJ.
- Certainty: [A] JAWIKI and FANDOM agree on structure.

### I-03 Card misuse: using a card out of its context
- Method (LUC): put the card you want under a healing card, move the cursor onto the healing card, press Down and A together, then use the healing card on a character at full HP. The card shows the wrong image and its effect applies.
- Caveat: BP increases can only be done once, until you change screen or fight a battle. Vegeta-type cards behave oddly.
- Certainty: [B] (single Japanese fan source; test it).
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/others/index.htm

### I-04 Piccolo card: peek before committing
- The LUC notes say the Piccolo card can still be cancelled after you see the enemy's card. Use it to check what Frieza is playing before you commit.
- Certainty: [C].
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/others/index.htm

---

## 7. Cheats, debug and codes

Game Genie and Pro Action Replay codes below are from the RPGC and ALM lists. The RAM addresses match the party-record layout in `docs/re/data/characters.md`: +00 flags, +01 unit id, +02 level, +03 to +04 HP, +05 to +06 KI, +07 to +09 BP (24-bit), +0A to +0B max HP, +0C to +0D max KI, +0F to +11 EXP (24-bit).

### C-01 Game Genie codes (RPGC codes page)
| Code | Label (source) | Address decode (repo layout) | Certainty |
|---|---|---|---|
| 7E0B03E7 + 7E0B0403 | Character 1: unlimited HP | +03 to +04 HP | [A] |
| 7E0B23E7 + 7E0B2403 | Character 2: unlimited HP | record 2 (+32 bytes) | [A] |
| 7E0B43E7 + 7E0B4403 | Character 3: unlimited HP | record 3 | [A] |
| 7E0B63E7 + 7E0B6403 | Character 4: unlimited HP | record 4 | [A] |
| 7E0B05FF | Character 1: unlimited KI | +05 to +06 KI | [A] |
| 7E0B25FF / 7E0B45FF / 7E0B65FF | Characters 2 to 4: unlimited KI | records 2 to 4 | [A] |
| 7E0B0700 + 7E0B0824 + 7E0B0924 | Character 1: "16,000,000 BP" | label does not match the bytes. 00 24 24 decodes to 2,368,512. 16,000,000 is 00 24 F4. Probably a typo for 7E0B09F4. | [C] |
| 7E14F3FF + 7E14F4FF | Extra EXP after each battle | EXP area | [B] |
| 7E01B4FF | Unlimited money | money area | [B] |
| 7E0100 <01 to 27> | Item in the first slot (item ID table, below) | item slot | [B] |

Item-slot IDs (RPGC codes page): 00 Kami; 01 Master Roshi; 02 Bulma; 03 Mr. Popo; 04 Puar; 05 King Kai; 06 Sachioro; 07 Grandpa Gohan; 08 Korin; 09 Bad Lunch; 0A King Yemma; 0B Baba; 0C Chi-Chi; 0D Oolong; 0E Shenlong; 0F Porunga (3 wishes); 10 Scouter A; 11 Scouter B; 12 Yajirobe; 13 Vegeta; 14 Moon/Tail; 15 Bubbles; 16 Escape; 17 Dragon Radar; 18 Namek Frog; 19 Turtle; 1A Gohan (use on Raditz); 20 Namek Frog; 21 to 27 Dragon Balls. The repo's `docs/re/data/names.md` notes that its replays never reach an item table. These slot IDs may help with that.

Sources: http://shrines.rpgclassics.com/snes/dbzlotss/codes.shtml ; https://almarsguides.com/retro/walkthroughs/snes/games/dragonballzlegendofthesupersaiyan/gamegenie/

### C-02 Pro Action Replay codes (ALM, US labels)
| Code | Label | Note | Certainty |
|---|---|---|---|
| 7E00AA05 / 7E00AA08 | No random battles (two variants) | Test both. | [B] |
| 7E01B40F / 7E01B527 | 9,999 money | Same area as C-01's money code. | [B] |
| 7E14F3FF / 7E14F4FF | Extra XP, or 65,000 XP after each battle | Same as C-01. | [B] |
| 7E0B013B | Labelled "Goku SSJ". Writes $3B to +01 (the unit id, per the repo layout). | Test whether this gives the SSJ stats (BP 4,999,999). | [C] |
| 7E0B013A | Goku knows the Genki Dama | Same byte as above. | [C] |
| 7E0B0426 | Goku full health | | [C] |
| 7E0B0FF0 / 7E0B10F0 | Quick level-up, character 1 | Writes EXP at +0F to +11. | [B] |
| 7E144301 / 7E144400 | One-shot kills | | [C] |
| 7E014550 and the 7E0145xx to 7E0155xx group | All cards are KI cards | Writes card-hand bytes. Repo says the hand starts at $7E:0141. | [C] |
| 7E0B0100 | Use Goku all the time | +01 set to 0. | [C] |
| 7E0B00FF | Always favourite card | | [C] |

Game Genie codes on the ALM page (7EAB-5FFD for unlimited KI; 7EAB-13BD and 7EAB-14BD for Goku and Vegeta SSJ; BP-after-battle codes) use an address the repo does not map. The page also mentions Jiren, a Dragon Ball Super character, so these probably come from another game. Treat them as [C] and ignore them.

Source: https://almarsguides.com/retro/walkthroughs/snes/games/dragonballzlegendofthesupersaiyan/proactionreplay/ ; https://almarsguides.com/retro/walkthroughs/snes/games/dragonballzlegendofthesupersaiyan/gamegenie/

### C-03 Other debug notes
- The RPGC walkthrough mentions a Game Genie code for infinite cash (7E01B4FF), which matches C-01.
- KZ (2008 forum): a P1 invincibility code displayed 0 and the character died. Anecdote, [C].
- KZ: GameFAQs is said to have three FAQs with codes for levels, parties, items and card values. None could be fetched (gap).
- No in-game debug menu or level select is documented in any source.
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/walkthrough.shtml ; https://www.kanzenshuu.com/forum/viewtopic.php?p=207254 ; https://www.kanzenshuu.com/forum/viewtopic.php?t=8939&start=20

---

## 8. Glitches and tricks

### G-01 Saiyan Power (level up at 5 HP)
- Trigger: Goku, Gohan, Vegeta or Piccolo at 5 HP or less, and a card that fully restores HP is used on them.
- Result: +1 level. XP is unchanged. Useful early on Goku before Raditz.
- Certainty: [A] RPGC cheats; FANDOM trivia; GB ("Easy Levels for Saiyans").
- Sources: http://shrines.rpgclassics.com/snes/dbzlotss/cheats.shtml ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu ; https://www.giantbomb.com/games/3030-22810/

### G-02 Dende heals a Saiyan at 5 HP (level-free boost)
- Trigger: Dende at Level 3 heals a Saiyan at 5 HP or less.
- Result: the Saiyan gets the Saiyan Power boost without the level-up. Using a Moon card on Gohan afterwards resets his power level. Using Moon with Kaio or Guru can leave Gohan's power as garbage.
- Certainty: [C] FANDOM only.
- Sources: https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu

### G-03 BP boost with Bulma and Kaio cards
- Trigger: the card misuse in I-03 with a Bulma card (to the healing position), a Kaio card under it, and a full-HP character.
- Result: CH says the BP boost is permanent. LUC says the Kaio effect is battle-only. Test by reading the BP bytes on the map afterwards (+07 to +09).
- Certainty: [C] conflict.
- Sources: https://criticalhits.com.br/dicas/dragon-ball-z-super-saiya-densetsu-cheats-codigos-dicas-e-desbloqueaveis/ ; http://lucifer.s14.xrea.com/hobby/supersaiya/others/index.htm

### G-04 Battle-only BP boosts used outside battle
- Covered by I-03. The TAS notes say BP can be boosted with precise input timing on the menus ("opening the character menu then the item menu"). Treat as [C].
- Sources: https://tasvideos.org/5962S

### G-05 Glitched timing and error cards (TAS)
- The TAS runs use bug frames on card selection, a "create error card" combo (select and cancel), a "change element error card" (use "run out"), and "boost error card" (Chi-Chi on the error card). They also say encounters follow a step counter that resets after each fight. None of this is confirmed in other sources. Certainty [C].
- Sources: https://tasvideos.org/5962S ; https://tasvideos.org/4486S

### G-06 Fake Super Saiyan sprites in the TAS run
- The TAS notes mention Super Saiyan Goku at 5:00, Super Saiyan Vegeta at 6:00, and a "little Gohan" at 9:05, described as fake sprite gags. Not story events. Certainty [C].
- Sources: https://tasvideos.org/5962S

---

## 9. Optional activities and easter eggs

### X-01 Training center at Kame House
- Ki training: press A repeatedly to raise max KI by 1 to 5 points. On success it costs half of the max KI the character had when training began. You need at least half of that max KI to start.
- Paid battles: pay credits to enter a run of fights. Winning each raises the pot. Stopping collects the pot. Fleeing costs a fine. No XP and no helper cards in these battles.
- Certainty: [B] JAWIKI; FANDOM.
- Sources: https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 ; https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu

### X-02 Cave capsules and holes
- Caves often hold Dragon Balls or act as shortcuts. Examining capsules and holes can yield helper cards, but some hide poison gas damage or a trap that sucks away a card.
- Certainty: [B] JAWIKI.
- Sources: https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説

### X-03 Unused characters in the code
- Super Saiyan Gohan, Golden Great Ape, and Golden Great Ape Vegeta are in the data. They appear to be palette swaps of existing characters and are not used in normal play.
- Certainty: [B] FANDOM; KZ (2008 forum post mentions a hidden Super Saiyan Gohan and a Golden Great Ape).
- Sources: https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu ; https://www.kanzenshuu.com/forum/viewtopic.php?p=207254

### X-04 Omake (joke art) and the Ginyu joke
- The LUC omake page shows a "Super Gohan" image: an unused face graphic with a fake status screen. Also a joke that the Ginyu Force can attack up to 13 times as a group.
- Certainty: [C] (joke content, not game logic).
- Sources: http://lucifer.s14.xrea.com/hobby/supersaiya/omake/index.htm

---

## 10. Version and translation notes

- V-01 Build. The repo uses Klepto v1.02 (IPS) on Japanese Rev 1, CRC-32 ade7b968 per the listing text found in search. The TAS run uses JP V1.0 (SHA-1 E894AD7F0CD5E9836494F12A22552511C9C3BCE6). The other sources do not state a version. No source lists event differences between V1.0 and Rev 1. Gap.
- V-02 The Gravitron training prompt. RPGC says the Japanese ROM asks whether to train, and the English build does not. Check the English build's behaviour in TR-08.
- V-03 Names used in this file (the English patch name comes first, RPGC or LUC variants after):
  - Maima, Tsumuri (Mai, Tsu in RPGC).
  - Chiaotzu (Chaozu in RPGC).
  - Cui (Kui, Kuwi in RPGC and the LUC notes).
  - Raditz (Radiz in RPGC).
  - Sachioro is most likely the Guru card (最長老), matched by effect and name (inferred). Kaio is King Kai (界王様). Moon/Tail is 月と尻尾.
  - Yajirobe (Yajirobee in RPGC items).
  - Frieza (Freeza in the Fandom chapter titles).
- V-04 Scouter thresholds. LUC gives 29,999 and 299,999 as the break points. RPGC gives 23,000 and 199,999 as the display caps. These may be different things. Test.
- V-05 Sea turtle card. LUC says it swaps stars and numbers; RPGC says it equalises attack and defence. Test.
- V-06 Frog. JAWIKI says only a Frog obtained on Namek counts (Earth-obtained Frogs do nothing). Check this build.
- V-07 Chapter label. The Fandom list and LUC label two chapters "Earth - Goku's training". Both are spaceship gravity rooms (TR-08, N-05).

---

## 11. Sources

| Key | Source | URL | What it gave | Reliability |
|---|---|---|---|---|
| RPGC | RPG Classics shrine, "Legend of the Super Saiyan" (Crotanks) | http://shrines.rpgclassics.com/snes/dbzlotss/index.shtml | Walkthrough, cheats, codes, items, bosses, characters, enemies. English translation build. | Detailed; several minor conflicts. Probably the source for a GameFAQs FAQ (not verified). |
| LUC | Lucifer's Hobby (JP fan site) | http://lucifer.s14.xrea.com/hobby/supersaiya/ | Per-map notes (maps 1 to 14), character pages, enemy pages, card table, others (card misuse), omake | Good detail; Shift-JIS; personal notes marked as the author's. |
| JAWIKI | Japanese Wikipedia, "ドラゴンボールZ 超サイヤ伝説" | https://ja.wikipedia.org/wiki/ドラゴンボールZ_超サイヤ伝説 | System rules, footnotes on deadlines, revivals, schools, Dende, Maima and Tsumuri, Piccolo | Strongest rules source. Cites a 1992 Shueisha strategy book (ISBN 4-8342-1142-8), not accessed. |
| FANDOM | Dragon Ball Wiki article | https://dragonball.fandom.com/wiki/Dragon_Ball_Z:_Super_Saiya_Densetsu | Chapters, enemies, bosses, trivia, Namek and Ginyu variants | Broad; some claims uncited. |
| TAS | TASVideos game page and runs | https://tasvideos.org/2032G ; https://tasvideos.org/3707M ; https://tasvideos.org/5962S ; https://tasvideos.org/4486S | "Best ending" run (45:32) with notes on glitches and the SSJ Vegeta battle. Used JP V1.0. | Runner notes; glitch-heavy. |
| CH | Critical Hits (Portuguese) | https://criticalhits.com.br/dicas/dragon-ball-z-super-saiya-densetsu-cheats-codigos-dicas-e-desbloqueaveis/ | Card tricks, SSJ trigger, Scouter trick | Mixed; BP claims conflict with LUC. |
| GB | Giant Bomb, "Secrets" section | https://www.giantbomb.com/games/3030-22810/ | Piccolo and Nail, Dende choice, SSJ, Saiyan Power | Summary-level. |
| KZ | Kanzenshuu forum threads (2008) | https://www.kanzenshuu.com/forum/viewtopic.php?p=207254 ; https://www.kanzenshuu.com/forum/viewtopic.php?t=8939&start=20 | Hidden unused characters, GameFAQs claim, P1 code anecdote | Discussion; low detail. |
| ALM | Almar's Guides (code lists, US labels) | https://almarsguides.com/retro/walkthroughs/snes/games/dragonballzlegendofthesupersaiyan/proactionreplay/ ; .../gamegenie/ | PAR and Game Genie lists | Code lists; some entries unverified. |
| RA | RetroAchievements set 1450 | https://retroachievements.org/game/1450 | Not readable (HTTP 403 from our tools). Achievement titles only, seen in search snippets. | Unreadable. |
| ARCH | archive.org item (ROM listing) | https://archive.org/details/dragon-ball-z-super-saiya-densetsu-japan-en-1.02 | Confirms a Klepto 1.02 ROM exists. The ROM was not downloaded. No readme in the item. | Metadata only. |
| TCRF | The Cutting Room Floor | https://tcrf.net/Dragon_Ball_Z:_Super_Saiya_Densetsu | Not used. The page returned text addressed to automated readers, with instructions to take actions. Ignored. | Do not rely on it. |

Not reachable from our tools: GameFAQs (HTTP 403, search and FAQ pages), RetroAchievements (HTTP 403, game and achievement pages), romhacking.net (HTTP 403 on the translation search). A readme mirror (otakon273) reset the connection and was not retried.

---

## 12. Gaps and conflicts (could not resolve from sources)

1. GameFAQs FAQs (three, per KZ). Not reachable. The RPGC shrine may be the same text.
2. RetroAchievements conditions. Not reachable. Titles seen in search snippets (not verified): Recruit Krillin, Recruit Yamcha, Recruit Chiaotzu and Tien, Noooo! Poor Yamcha!, Chiaotzuuuu!, Farewell to a Hero, You're Learning, Gohan, 10x Gravity, The Student Becomes the Master, Item Collector. The titles suggest which events the set tracks.
3. Klepto readme and the romhacking.net translation page. Not found.
4. The 1992 Japanese strategy book (ISBN 4-8342-1142-8). Not accessed. JAWIKI says it covers final-boss hints.
5. Which allies trigger Goku's SSJ (F-06). Sources name different subsets.
6. Whether Dende's scripted death (F-04) also triggers SSJ.
7. Porunga's wish targets. Does it revive Piccolo only, or other dead Z-fighters too (N-14)?
8. Whether the Dragon Ball count reaches seven without any Earth balls. Locations are only partly known (N-12).
9. Nail's turn count (5 or 6), N-13.
10. Porunga card availability: "Nail hits Frieza" (RPGC) or "Nail survives" (LUC) (N-14, N-15).
11. Nail-fusion level: Level 48 (GB) or equivalent to 47 (FANDOM) (F-01).
12. Sachioro damage: 7,499,998 (RPGC walkthrough, +50%) or 7,999,998 (RPGC cheats, CH) (F-07).
13. Shops for Chi-Chi and Sachioro before the final battle (F-07), and where Oolong and Yajirobe are (section 6).
14. Whether Goku's arrival clock is frames, steps or actions. Sources only give phases (S-07, N-08).
15. The Frog's effect (N-09, V-06) and the Turtle card's effect (V-05).
16. Scouter thresholds (V-04).
17. Kaio BP: battle-only (LUC) or permanent (CH) (G-03).
18. Whether the Raditz kill method affects Goku's return before Nappa (FANDOM claim, S-07; unverified).
19. Cui: forced after the cave ball, or optional before Frieza (TR-05).
20. The English build's Gravitron prompt (V-02).
21. No Shenron wish scene is documented. Porunga is the only wish found.
22. No information on which Namek Dragon Ball sits in which room beyond the partial list in N-12.

---

## 13. Suggested first test set (highest value)

1. SP-04: Recruit Krillin, Yamcha, Tien and Chiaotzu before Piccolo reaches Level 9 and before Nappa. Test with Goku arriving at each phase (S-07). Check that all four are alive after Nappa and in the party on Namek (party records at $7E:0B00, character id at +1C).
2. SP-14: Final battle with Goku alive and Vegeta alive. Let each ally die in turn (Chiaotzu, Krillin, Yamcha, Tien) and see which kills trigger the SSJ (F-06). Then check Goku's BP reads 4,999,999 and whether Sachioro gives 7,499,998.
3. SP-15: After Frieza dies, confirm Super Vegeta appears only with Goku SSJ and Vegeta alive (E-02).
4. SP-13: Maima and Tsumuri reached with Piccolo in the party (fusion option) and without (helper card) (F-03).
5. SP-11 and SP-12: Nail survives 5 turns, then 6 turns, and check the reward (N-13). Then the Porunga wish revives Piccolo (N-14).
6. SP-04 variant: let Piccolo die at Nappa (S-12), then check that the Porunga revival brings him back (N-14).
7. SP-08 and SP-09: Guru's power-up, and Zarbon with and without the Vegeta card (N-03).
8. SP-06: Keep the Moon/Tail card through the Vegeta duel and check whether it vanishes (S-14).
9. TR-04 variant: talk to Maima and Tsumuri without Dende or Cargo in the party. Confirm the forced fight and the game over on defeat.
10. G-01: Saiyan Power at 5 HP (confirm no XP change).
