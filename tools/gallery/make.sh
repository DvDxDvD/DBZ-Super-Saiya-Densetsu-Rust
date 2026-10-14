#!/usr/bin/env bash
# Makes the pictures of the README's comparison gallery (docs/media/cmp-*.png) from your own ROM.
#
#   tools/gallery/make.sh [--exe target/release/ssd-recompiled.exe] [--replays .work/replays] [--out docs/media]
#
# Needs the game with its ROM cached (README, "Getting started"), the HD pack installed (the Remastered
# pictures use it), and the reference replays (`cargo run --release -p xtask -- make-replays`, and
# tools/saves/secrets.sh for the `secret-*` ones). tools/gallery/frames.txt says what each picture is.
# The game draws offscreen (--ui-snapshot): no window is opened. The PNGs the game writes are stored
# uncompressed; with Python and Pillow `finish.py` removes the snapshot's small label, recompresses them and
# makes banner.png.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exe="$root/target/release/ssd-recompiled.exe"
replays="$root/.work/replays"
out="$root/docs/media"
while [ $# -gt 0 ]; do
    case "$1" in
        --exe) exe="$2"; shift 2 ;;
        --replays) replays="$2"; shift 2 ;;
        --out) out="$2"; shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 64 ;;
    esac
done
mkdir -p "$out"

grep -v '^[[:space:]]*#' "$root/tools/gallery/frames.txt" | grep -v '^[[:space:]]*$' |
while IFS='|' read -r name replay classic remastered script; do
    name="$(echo "$name" | xargs)"; replay="$(echo "$replay" | xargs)"
    classic="$(echo "$classic" | xargs)"; remastered="$(echo "$remastered" | xargs)"
    script="$(echo "$script" | xargs)"
    remastered="${remastered:-$classic}"
    echo "== $name ($replay, frames $classic / $remastered${script:+, $script})"
    "$exe" --replay "$replays/$replay.ssdreplay" --headless-frames "$classic" \
        --ui-size 1170x956 --ui-snapshot "$out/cmp-$name-classic.png"
    args=(--replay "$replays/$replay.ssdreplay" --headless-frames "$remastered"
          --mode enhanced --hd-scale 4 --ui-size 1488x956 --ui-snapshot "$out/cmp-$name-remastered.png")
    if [ -n "$script" ]; then args+=(--ui-script "$root/tools/gallery/$script"); fi
    "$exe" "${args[@]}"
done

if command -v python >/dev/null 2>&1 && python -c "import PIL" >/dev/null 2>&1; then
    python "$root/tools/gallery/finish.py" "$out" --banner
else
    echo "Pillow is not installed: the pictures keep the game's uncompressed PNGs and the label of the snapshot" >&2
fi
echo "Pictures in $out"
