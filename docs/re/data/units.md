# Units (enemies and other fighters), their names, commands and encounters

Tables: `unit_stats` (02:B214), `unit_name_pointers` (02:ACE8), `unit_command_pointers`
(02:B7D2), `unit_command_lists` (02:B8A4), `encounter_groups` (02:BF0B). Read by `ssd-catalogue`
(`Enemy`, `Encounter`). The unit id is the index in every one of them.

## Stats row (14 bytes)
Found from the routine at 01:F719, which builds the battle record of each enemy from this row, and
confirmed against the enemy record of the reference battle (a Saibaman, unit 4).

| Byte | Meaning |
|---|---|
| 0 | the fighter's *class* (0 to 6), copied to record byte +1B: a card whose symbol equals it (or is 7) gives the fighter 25 percent more battle power (`cards.md`, `battle.md`) |
| 1 | level |
| 2..3 | HP |
| 4..6 | BP (24 bits); the record gets the row's BP minus its two low decimal digits read as one byte, plus or minus a small figure (01:F719; `battle.md`) |
| 7..8 | KI |
| 9 | copied to record byte +17 (about 30): the *defence factor* of the damage formula (`battle.md`) |
| 10 | copied to record byte +0E (1 to 16) |
| 11..13 | EXP reward, 24 bits (1D:8C33 adds it to the battle's total when the enemy is defeated; the third byte is always 0): the reference battle gives Goku 9 EXP, the row's value |

The record layout is the one of the party record (`characters.md`); enemy records are five of 32
bytes from `$7E:0E00` (the byte at +01 is the unit id; `entity_ids` is that id byte of each).
Rows 0 and 1 and the other fighters of the party (ids 9 to 13 and so on) are all zero: their stats
come from the character tables. There are 105 rows; some are story forms of one fighter (four
Friezas, two Zarbons...).

## Names
`unit_name_pointers` has one word per unit id, each pointing at a string of bank 02 (a length byte
and that many tile codes, see `names.md`). Units 82 and 85 to 95 point at data that is not a
name; the rest are English.

## Commands
`unit_command_pointers` leads to one list per unit in `unit_command_lists`; the battle menu code
(03:876C) reads them with the party member's unit id and the cursor position, counting entries
that are not `$FF`. A list starts with a count and a zero, then groups of ids separated by `$FF`,
then one word per command. The groups are not decoded; the catalogue keeps the bytes.

## Encounter groups
`encounter_groups` has 112 groups of five unit ids (`$FF` = empty). The routine at 01:F692 copies a
group into the five enemy slots; the group number is `encounter_group_index` (`$7E:0EA3`). The slots are screen
positions and the group's count decides which are used (1 enemy: slot 0; 2: slots 1 and 2; 3: 0, 3 and 4; 4: 1 to 4; 5: all);
`battle.md` has the rest (how the records are built, where to scale them, which random zone picks which group). The
reference battle uses group 3: one Saibaman in the first slot.

### Groups the story starts (`scenes.md`)

Scenes start battles by group number; the groups are in the catalogue under the name of their first unit
(the story forms share names). Seen in the crafted-save replays (`../secrets-coverage.md`): `$05` Raditz,
`$14` Nappa (and `$15` Vegeta), `$12` and `$13` the Saimen of the Saiyan chapter's start, `$24` Cui, `$25`
Dodoria, `$38` Zarbon, `$4D` Recoome and Guldo, `$4E` Jeice and Burter, `$51` Ginyu, `$53` Frieza's first form
(unit 53), `$54` to `$56` his second, third and final forms (units 54 to 56), `$57` Frieza against Nail (unit
78, 400 000 BP), `$59` a third-form Frieza (unit 100), `$6D` the second Nail (unit 103), `$6F` Super Vegeta
(unit 75, the hidden boss). A won battle of `$56` or `$6E` ends the game (`scenes.md`, "The last chapter").
