#!/usr/bin/env bash
# Runs trace.lua in headless Mesen2 (portable instance, script I/O access enabled).
# Use (Git Bash, from anywhere):
#   tools/re/run_trace.sh "<rom.sfc>" <run_name> [frames=36000] [mode=mash|walk|auto|explore|battle|idle]
# It first runs `cargo run --release -p xtask -- export-mlb`, which writes dbz.mlb and symbols.lua
# into .work/re/, then runs the tracer with those symbols.
# Environment:
#   MESEN_DIR   portable Mesen2 instance outside the repo (required; for example
#               C:/Tools/re-dbz/mesen-headless); use a copy per run to launch several runs in parallel.
#   DBZ_SEED (auto), DBZ_ROUTE (explore), DBZ_INPUTS, DBZ_SHOT_DIR, DBZ_SHOT_EVERY, DBZ_TRACE, ...
#               passed through to trace.lua (see its header).
# Output in .work/re/<run_name>/ (exec.csv, calls.csv, jumps.csv, prgrom.cdl, summary.txt).
# Every run starts clean: SRAM, savestates and CDL accumulated by Mesen are removed before and after.
set -euo pipefail

if [ $# -lt 2 ]; then
  echo "usage: tools/re/run_trace.sh <rom.sfc> <run_name> [frames] [mode]" >&2
  exit 64
fi
ROM="$1"
RUN="$2"
FRAMES="${3:-36000}"
MODE="${4:-mash}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
MESEN="${MESEN_DIR:?set MESEN_DIR to a portable Mesen2 instance (it is cleaned before and after each run)}"
if [ ! -f "$MESEN/Mesen.exe" ]; then
  echo "MESEN_DIR=$MESEN does not hold Mesen.exe; refusing to clean it" >&2
  exit 1
fi
WORK="$ROOT/.work/re"
export PATH="$HOME/.cargo/bin:$PATH"

clean() { rm -rf "$MESEN/Debugger" "$MESEN/Saves" "$MESEN/SaveStates" "$MESEN/Satellaview"; }
clean
trap clean EXIT

(cd "$ROOT" && cargo run --release -q -p xtask -- export-mlb "$WORK" >&2)
mkdir -p "$WORK/$RUN"
export DBZ_FRAMES="$FRAMES" DBZ_MODE="$MODE" DBZ_OUT="$(cygpath -m "$WORK/$RUN")/" \
  DBZ_SYMBOLS="$(cygpath -m "$WORK/symbols.lua")" DBZ_SHOT_DIR="${DBZ_SHOT_DIR:-}"
# Mesen timeout in seconds: ~150 frames/s with the exec callback on, plus margin
"$MESEN/Mesen.exe" --testRunner "$(cygpath -w "$HERE/trace.lua")" "$(cygpath -w "$ROM")" \
  --timeout=$(( FRAMES / 60 + 300 )) > /dev/null 2>&1 || true
cat "$WORK/$RUN/summary.txt"
