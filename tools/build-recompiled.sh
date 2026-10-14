#!/usr/bin/env bash
# Recompiles the game from your ROM and builds it with the recompiled code, from Git Bash.
#
#   tools/build-recompiled.sh [--rom <file> [--ips <dbz102.ips>]] [--out <dir>] [--zip]
#
# The recompiler (crates/ssd-recomp) turns the 65816 code of YOUR ROM into Rust, written to
# .work/recomp/ (never tracked, never shared: it is derived from the game), and the game is built
# with the `recomp` feature so that code runs instead of the interpreted original. The launchers of
# the folder start the game with --hooks rust. It takes a while (the generated source is about 24 MB;
# expect ten minutes or more to compile). Needs the Rust toolchain, CMake and the MSVC build tools,
# and your ROM: give it with --rom the first time (the game keeps a clean copy under
# %LOCALAPPDATA%\dbz-ssd-recompiled\ and reads it from there afterwards).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

rom=""; ips=""; out="dist/dbz-ssd-recompiled-windows-recompiled"; zip_flag=""
while [ $# -gt 0 ]; do
    case "$1" in
        --rom) rom="$2"; shift 2 ;;
        --ips) ips="$2"; shift 2 ;;
        --out) out="$2"; shift 2 ;;
        --zip) zip_flag="--zip"; shift ;;
        *) echo "Unknown argument: $1" >&2; exit 64 ;;
    esac
done

data="${SSD_DATA_DIR:-${LOCALAPPDATA:-.}/dbz-ssd-recompiled}"
cache="$data/cache/rom/canonical.sfc"

# 1. The tools, and your ROM in the cache
cargo build --release -p ssd-app -p xtask
if [ -n "$rom" ]; then
    args=(--rom "$rom" --headless-frames 1)
    if [ -n "$ips" ]; then args+=(--ips "$ips"); fi
    target/release/ssd-recompiled.exe "${args[@]}"
fi
if [ ! -f "$cache" ]; then
    echo "No ROM yet: run this script again with --rom <your rom> (and --ips <dbz102.ips> for the Japanese ROM)." >&2
    exit 1
fi

# 2. The recompiler: your ROM to Rust, under .work/recomp/
cargo run --release -p xtask -- recompile

# 3. The game, with the generated code
cargo build --release -p ssd-app --features recomp

tools/windows/assemble.sh target/release/ssd-recompiled.exe "$out" "--hooks rust" $zip_flag
echo "The recompiled build is for your own use: the generated code is derived from your ROM, do not share it."
