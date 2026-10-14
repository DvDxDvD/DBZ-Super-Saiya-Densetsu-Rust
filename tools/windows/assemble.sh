#!/usr/bin/env bash
# Lays out the Windows folder of the game: the program, the launchers, the quickstart and the
# licences (what the prebuilt download holds). Used by build-classic.sh and build-recompiled.sh.
#
#   assemble.sh <ssd-recompiled.exe> <out dir> [extra launcher arguments] [--zip]
#
# The extra launcher arguments are '--hooks rust' for a build with the recompiled code, and nothing
# otherwise. hd-pack.zip is a separate download that the player puts beside the program.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"

exe="${1:?the built ssd-recompiled.exe}"
out="${2:?the folder to make}"
hooks="${3:-}"
zip_it=""
if [ "$hooks" = "--zip" ]; then hooks=""; zip_it=1; fi
if [ "${4:-}" = "--zip" ]; then zip_it=1; fi

[ -f "$exe" ] || { echo "The program is not there: $exe" >&2; exit 1; }
rm -rf "$out"
mkdir -p "$out/licenses"

cp "$exe" "$out/ssd-recompiled.exe"
cp "$root/re/symbols.toml" "$out/symbols.toml"

# The launchers and the quickstart are written with Windows line ends
crlf() { tr -d '\r' | sed 's/$/\r/'; }
for name in "Play Classic.bat" "Play Remastered.bat" "Set up my ROM.bat"; do
    tr -d '\r' < "$here/$name" | sed "s|@HOOKS@|$hooks|" | crlf > "$out/$name"
done
crlf < "$here/QUICKSTART.txt" > "$out/QUICKSTART.txt"

for candidate in "$root/LICENSE" "$root/tools/public/LICENSE"; do
    if [ -f "$candidate" ]; then cp "$candidate" "$out/LICENSE.txt"; break; fi
done
for candidate in "$root/NOTICE.md" "$root/tools/public/NOTICE.md"; do
    if [ -f "$candidate" ]; then cp "$candidate" "$out/NOTICE.md"; break; fi
done
cp "$root/third_party/jgenesis/LICENSE" "$out/licenses/jgenesis-GPL-3.0.txt"
cp "$root"/assets/fonts/*-OFL.txt "$out/licenses/"

if [ -n "$zip_it" ]; then
    archive="$out.zip"
    rm -f "$archive"
    if command -v zip >/dev/null 2>&1; then
        (cd "$out" && zip -qr "$(cd "$(dirname "$archive")" && pwd)/$(basename "$archive")" .)
    elif command -v powershell.exe >/dev/null 2>&1; then
        powershell.exe -NoProfile -Command "Compress-Archive -Path '$(cygpath -w "$out")\\*' -DestinationPath '$(cygpath -w "$archive")'"
    else
        echo "No zip tool found (zip or powershell.exe); the folder is laid out without the archive." >&2
        exit 1
    fi
    echo "Wrote $archive"
fi
echo "Laid out $out"
