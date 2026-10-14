#!/usr/bin/env bash
# Crafted saves and replays that reach the hidden and conditional events of the game.
# See docs/re/secrets-coverage.md for what each one shows.
#
#   tools/saves/secrets.sh BASE.sav [name ...]
#
# BASE.sav is any battery save with a used slot 1 (any first-chapter save works; it is only
# read). Every recipe edits a copy of that slot (party, levels, map, position, story flags) with
# `xtask save-edit` and plays a timeline of tools/saves/secrets/ from Continue with `xtask
# replay-from-save`. The saves go to .work/demo/secret-<name>.sav and the replays to
# .work/replays/secret-<name>.ssdreplay (neither is ever tracked). Run from the workspace root,
# with the ROM cached.
set -euo pipefail
base=${1:?usage: tools/saves/secrets.sh BASE.sav [name ...]}
shift || true
x="cargo run --release -q -p xtask --"
[ -x target/release/xtask.exe ] && x=target/release/xtask.exe
[ -x target/release/xtask ] && x=target/release/xtask
mkdir -p .work/demo .work/replays
only=("$@")

# make NAME SCRIPT SET...: the save of the recipe, then the replay of the script from it.
make() {
    local name=$1 script=$2
    shift 2
    if [ ${#only[@]} -gt 0 ] && ! printf '%s\n' "${only[@]}" | grep -qx "$name"; then return; fi
    local sets=()
    for s in "$@"; do sets+=(--set "$s"); done
    $x save-edit "$base" ".work/demo/secret-$name.sav" "${sets[@]}" > /dev/null
    $x replay-from-save ".work/demo/secret-$name.sav" "tools/saves/secrets/$script.txt" \
        ".work/replays/secret-$name.ssdreplay"
}

# Chapter 1 (Earth): Goku and Piccolo from the first-chapter save, level up, flag 3 set.
make raditz mash-3000 goku.level=12 piccolo.level=12 flags=+3

# Chapter 4 (Earth, the Saiyans): map 5, Gohan and Piccolo, standing next to the recruits.
earth=(party=gohan,piccolo gohan.level=8 piccolo.level=8 map=5 flags=none,+1,+2 scene=0)
make recruit-krillin recruit-krillin "${earth[@]}" leader=680,690 follower=552,570
make recruit-yamcha recruit-yamcha "${earth[@]}" leader=920,940 follower=800,820
make recruit-tien-chiaotzu recruit-tien-chiaotzu "${earth[@]}" leader=1320,1784 follower=1192,1664
# The Saiyans themselves: stronger fighters (level 30) so the scene goes on past Nappa's fight.
make saiyan-nappa saiyan-nappa "${earth[@]}" gohan.level=30 piccolo.level=30 leader=1316,1200     follower=1188,1080

# Namek, first chapter there (map 11): the five Z-fighters of the Earth chapter all arrived.
make namek-zfighters status party=gohan,krillin,tien,yamcha,chiaotzu gohan.level=24 \
    krillin.level=22 tien.level=22 yamcha.level=20 chiaotzu.level=18 map=11 \
    flags=none,+1,+3,+4,+5,+6,+0x10,+0x11,+0x12,+0x13 leader=768,424 follower=640,304 \
    items=d.radar,scouter zeni=300 scene=0
# Zarbon and Vegeta joining: the same five on map 11, flag 2 set (the scene's test), flag 0x1B set
# (else the scene of map 11's other trigger replaces it).
make zarbon-vegeta-joins mash-3600 party=gohan,krillin,tien,yamcha,chiaotzu gohan.level=30 \
    krillin.level=28 tien.level=28 yamcha.level=26 chiaotzu.level=24 map=11 \
    flags=none,+1,+2,+3,+4,+5,+6,+0x10,+0x11,+0x12,+0x13,+0x1B leader=768,1000 \
    follower=640,880 scene=0

# Namek, map 9: the rescue of Dende (position inside the village), then the flying chase.
make dende-rescue mash-6000 party=gohan,krillin gohan.level=20 krillin.level=20 map=9 \
    flags=none,+1,+3 leader=880,320 follower=752,200 scene=0
# Cui, with Goku in the party (flag 0) and the "after the cave" flag 0x1E.
make cui mash-3000 party=goku,gohan,krillin goku.level=40 gohan.level=35 krillin.level=32 map=9 \
    flags=none,+0,+1,+3,+0x1E leader=1200,1000 follower=1072,880 scene=0
# Dodoria at the ship: Goku, Krillin and Vegeta present, flag 0x1B, at the landing place.
make dodoria mash-3000 party=goku,gohan,krillin,vegeta goku.level=40 gohan.level=35 \
    krillin.level=32 vegeta.level=36 map=9 flags=none,+0,+1,+3,+7,+0x1B leader=336,896 \
    follower=208,776 scene=0
# Maima and Tsumuri on map 9, talked to with Piccolo and Dende in the party: the fight with them
# (Dende rests in it).
make maima-tsumuri talk-maima party=piccolo,gohan,dende piccolo.level=47 gohan.level=40 \
    dende.level=3 map=9 flags=none,+1,+2,+0xB leader=2436,624 follower=2308,504 scene=0
# Map 34: Nail joins and fights the other Nail (flag 3, Krillin, must be clear).
make fake-nail mash-3000 party=gohan,vegeta gohan.level=35 vegeta.level=36 map=34 \
    flags=none,+1,+7 leader=120,150 follower=0,30 scene=0

# The Ginyu squad's chapter, map 16.
ginyu=(party=gohan,krillin,vegeta gohan.level=30 krillin.level=27 vegeta.level=28 map=16 scene=0
    items=d.radar,scouter)
make ginyu-jeice-burter mash-3600 "${ginyu[@]}" flags=none,+1,+3,+7 leader=532,1148 follower=404,1028
make ginyu-goku-duel mash-4800 party=goku,gohan,krillin,vegeta goku.level=40 gohan.level=30 \
    krillin.level=27 vegeta.level=28 map=16 scene=0 items=d.radar,scouter \
    flags=none,+0,+1,+3,+7,+0x1F,+0x12 leader=1860,800 follower=1732,680
# Nail against Frieza, the dragon balls and Porunga: all three "Ginyu done" flags set; Dende has
# a record (the scene makes him join: without one his record would be noise from the image).
make nail-porunga mash-8400 party=gohan,krillin,vegeta,dende gohan.level=30 krillin.level=27 \
    vegeta.level=28 dende.level=3 party=gohan,krillin,vegeta map=16 scene=0 \
    items=d.radar,scouter flags=none,+1,+3,+7,+0x1F,+0x22,+0x1A leader=1792,768 follower=1664,648

# Frieza on Namek: after the first random battle of map 22 with event bit 0 and five battles
# counted (page:83, page:B0) the first form attacks; on map 24, with Goku in the party and the
# counter at 3, the next random battle is Frieza's third form.
make frieza-forced-battle walk-fight party=gohan,krillin,vegeta gohan.level=35 krillin.level=32 \
    vegeta.level=36 map=22 flags=none,+1,+3,+7 page:83=01 page:B0=0500 leader=1500,1500 \
    follower=1372,1380 scene=0
make frieza-random-encounter walk-fight party=goku,gohan,piccolo,vegeta goku.level=47 \
    gohan.level=40 piccolo.level=40 vegeta.level=40 map=24 flags=none,+0,+1,+2,+7 page:B0=0300 \
    leader=2000,2000 follower=1872,1880 scene=0

# F-03, the fusion of Piccolo: map 24, Piccolo in the party (flag 1 of the scene's test is computed from the
# party records), flag 2 clear, Krillin (flag 3) absent so that Dende leaves, Vegeta (flag 7) present; the
# leader stands under the two Namekians (the objects of the map at x 1656 and 1672, y 1152) and talks.
make piccolo-dende-fusion talk-fusion party=piccolo,gohan,vegeta,dende piccolo.level=40 gohan.level=40 \
    vegeta.level=40 dende.level=3 map=24 flags=none,+1,+7,+0xB leader=1664,1172 follower=1536,1052 scene=0
# Piccolo's return after Frieza's first form: map 22, the roster has Tien, Yamcha and Chiaotzu outside the
# party, so the list the chapter's set-up keeps (image:1A0, ids ended by 0xFF) is theirs. The party is
# Piccolo then that list once the scene after the fight has run.
make piccolo-returns frieza-flees party=gohan,krillin,tien,yamcha,chiaotzu,vegeta gohan.level=35 \
    krillin.level=32 tien.level=30 yamcha.level=28 chiaotzu.level=26 vegeta.level=36 piccolo.level=40 \
    party=gohan,krillin,vegeta map=22 flags=none,+1,+3,+7 page:83=01 page:B0=0500 leader=1500,1500 \
    follower=1372,1380 scene=0 image:1A0=040506FF

# Namek, the last map (24): flag 8 and no battles counted, at the place of the last scene. The party
# list (image:2A0) and Frieza's hit points (image:303 and 30A, words) are what the scene copies.
# Piccolo and Krillin at 1 hit point: one of them dies, which makes Goku a Super Saiyan.
make ssj-frieza mash-7000 party=gohan,piccolo,krillin,vegeta,goku goku.level=47 gohan.level=40 \
    piccolo.level=40 piccolo.hp=1 krillin.level=30 krillin.hp=1 vegeta.level=40 map=24 \
    flags=none,+0,+1,+2,+3,+7,+8 page:B0=00 leader=2100,2592 follower=1972,2472 scene=0 \
    image:2A0=01020307FF image:303=3C00 image:30A=3C00
# Goku already a Super Saiyan (unit 0x3B, 4 999 999 BP) and Frieza at 1 hit point: the ending, the
# credits and, with Vegeta alive, the hidden fight after them.
make ending-super-vegeta mash-17000 party=gohan,piccolo,vegeta,goku goku.level=47 gohan.level=40 \
    piccolo.level=40 vegeta.level=40 image:01=3B goku.bp=4999999 map=24 \
    flags=none,+0,+1,+2,+7,+8 page:B0=00 leader=2100,2592 follower=1972,2472 scene=0 \
    image:2A0=010207FF image:303=0100 image:30A=D007
# The same ending without Vegeta in the party: the hidden fight needs him, so after the credits
# the game ends on the END screen.
make ending-normal mash-17000 party=gohan,piccolo,goku goku.level=47 gohan.level=40 \
    piccolo.level=40 image:01=3B goku.bp=4999999 map=24 flags=none,+0,+1,+2,+8 page:B0=00 \
    leader=2100,2592 follower=1972,2472 scene=0 image:2A0=0102FF image:303=0100 image:30A=D007
