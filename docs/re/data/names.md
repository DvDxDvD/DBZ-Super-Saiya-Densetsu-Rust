# Text: tile codes, string tables, attack and object names

## Encoding
Every string is bytes that are tile numbers of the font, not ASCII. The English translation draws
its letters from tiles that are interleaved with the Japanese kana, so the codes skip: lower-case
letters have codes in `05`-`5C` (and are not in alphabetical order), digits are `80`-`89`, space
is `01`. Capital letters come from two sets. `ssd-catalogue/src/text.rs` is the table. Control
codes in dialogue (not in names): `FF` ends a line, `7E` and `7F` are quotes, and the codes from
`A0` up are translated to tiles of another set when drawn (01:BC88). A message names things with
slots (01:D120) whose type is in the high bits of the slot's value: unit names, object names,
attack names.

How it was found: the dialogue was located by the pattern of repeated letters of known phrases
("special attack" has a doubled t and two a), the font was read from VRAM while the message was on
screen, and the letters were checked against the tile words the text engine writes into the tile
map buffer (the reference battle's "Piccolo uses a special attack on Saibai!").

## String tables in bank 02
A name is a length byte then that many tile codes. The tables of pointers (words, bank 02):

| Table | Entries | Indexed by |
|---|---|---|
| `string_pointers` (02:AB66) | 44 | string number: 12 character names, then menu words (HP, KI, Save?...) |
| `unit_name_pointers` (02:ACE8) | 105 | unit id |
| `attack_name_pointers` (02:AEA6) | 69 | attack id (Ki Blast, KameHameHa, Spirit Bomb, Heal...) |
| `object_name_pointers` (02:B0D9) | 40 | object id |

Several entries are repeated (one name for several ids) and a few point at nothing or at data that
is not text; the catalogue gives those an empty or unprintable name. Many attack names are still
Japanese kana, which have no entry in the tile table: they show as codes in angle brackets.

The object names hold the story's people (Kami, Roshi, Bulma...) and its items: Scouter, Radar,
Turtle and the seven dragon balls. The game has no separate item table that the replays reach (the
Item menu is empty in them); the catalogue's items are this table.
