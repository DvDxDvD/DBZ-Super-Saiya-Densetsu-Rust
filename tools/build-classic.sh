#!/usr/bin/env bash
# Builds the game without the recompiled code and lays out the Windows folder (the prebuilt
# download), from Git Bash.
#
#   tools/build-classic.sh [out dir] [--zip]      (out dir: dist/dbz-ssd-recompiled-windows)
#
# The Classic build: the original game runs in the emulator core, and the routines ported by hand
# (re/symbols.toml) are there for --hooks rust. It needs the Rust toolchain (rustup installs the
# pinned one), CMake and the MSVC build tools (SDL3 is built from source). No ROM is needed to build.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

out="dist/dbz-ssd-recompiled-windows"
zip_flag=""
for arg in "$@"; do
    case "$arg" in
        --zip) zip_flag="--zip" ;;
        *) out="$arg" ;;
    esac
done

cargo build --release -p ssd-app
tools/windows/assemble.sh target/release/ssd-recompiled.exe "$out" "" $zip_flag
