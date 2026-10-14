# Characters: level tables and the party record

Tables: `character_level_pointers` (02:C13B), `character_level_rows` (02:C155), `party_unit_ids`
(00:8AE5), and the names of `string_pointers` (02:AB66). Read by `ssd-catalogue` (`Character`,
`LevelRow`). Found by watching the writes to the party records at `$7E:0B00` while the reference
battle replay starts a new game, then reading the routines that make them.

## The party record
The party is nine records of 32 bytes from `$7E:0B00` (`party_records`). The bytes the replays use:

| Byte | Meaning |
|---|---|
| +00 | flags (`$C0`: in use) |
| +01 | unit id of the character (see below) |
| +02 | level |
| +03..04 | current HP |
| +05..06 | current KI |
| +07..09 | BP (24 bits) |
| +0A..0B | maximum HP |
| +0C..0D | maximum KI |
| +0E | a byte copied from the level row; also the tie-break when the battle picks who acts next |
| +0F..11 | EXP (24 bits) |
| +17 | 30 for the first two characters (a table value for units; not decoded for characters) |
| +1B | 1 for the first two characters |
| +1C | character id |

## Building a record
The routine at 00:89B8 clears the record, sets its level, then reads the row of the character's
level table through the pointer of its id (`character_level_pointers`, indexed by id; a table is
48 rows of 11 bytes, or fewer, see below) and stores it:

| Row byte | Goes to | How |
|---|---|---|
| 0..2 | BP | added to the record's BP |
| 3..5 | EXP | stored (the EXP at which this level begins; row 1 is 0) |
| 6..7 | max HP and HP | stored |
| 8..9 | max KI and KI | added to the record's KI |
| 10 | record byte +0E | stored |

Rows are meant to be applied level after level (BP and KI are gains), and the routine is called
with the starting level, so for level 1 the row is the starting value: Goku starts with 416 BP,
80 HP and 12 KI, Piccolo with 408, 77 and 12, which is what the reference battle shows.

A routine that follows (00:8A69) overrides the values for the character ids from 8 up, except 11:
their stats come from row `id + 26` of the unit table (`unit_stats`), the same rows the enemies
use. The pointer words of ids 8 to 10 and 12 lead into the first table and are not used.

## Which tables there are
Eight tables of 48 levels (ids 0 to 7, in order of their pointers, 528 bytes apart) and a ninth
for id 11 that holds only three rows and is followed by code: its levels are Dende's, who cannot
grow beyond level 3. The table size in `symbols.toml` ends with those three rows.

In some tables the HP (and in one the EXP) goes down at level 8. The cause is not known; the
values are read as stored.

## Names and ids
`party_unit_ids` maps a character id (0 to 11) to its unit id, the id used by the unit tables
(see `units.md`). The characters are Goku, Gohan, Piccolo, Krillin, Tien, Yamcha, Chiaotzu,
Vegeta, Nail, Tsumuri, Maima and Dende, in that order, named by the first twelve entries of the
string table. The unit name table gives each of them the same name but id 8, whose unit row (34)
is named Chiaotzu there.

The unit id of a character is byte +01 of its record; the battle code uses it to find the
commands of the fighter (`unit_command_pointers`).
