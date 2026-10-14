#!/usr/bin/env bash
# Creates/recreates the headless Ghidra project with the correct SNES layout, the Mesen2 trace
# applied and auto-analysis.
# Use (Git Bash):
#   tools/re/run_ghidra.sh "<rom.sfc>" [trace_dirs separated by '+'] [project_name]
# Required environment (current values of this machine are in README.md):
#   GHIDRA_HOME      Ghidra installation folder
#   JAVA_HOME        JDK used by Ghidra
#   GHIDRA_PROJECTS  folder holding the Ghidra projects (outside the repo)
# Optional environment:
#   GHIDRA_SETTINGS  alternative settings folder (e.g. with the patched ghidra-snes), passed as
#                    -Dapplication.settingsdir; empty = standard user settings.
#   DBZ_BASELINE=1   does not apply the trace (only the RESET/NMI/IRQ vectors), for comparison.
# Default traces: every run folder under .work/re/ of this repository.
set -euo pipefail

: "${GHIDRA_HOME:?set GHIDRA_HOME (Ghidra installation folder)}"
: "${JAVA_HOME:?set JAVA_HOME (JDK folder)}"
: "${GHIDRA_PROJECTS:?set GHIDRA_PROJECTS (folder of the Ghidra projects)}"
export JAVA_HOME

ROM="$1"
HERE="$(cd "$(dirname "$0")" && pwd -W)"
ROOT="$(cd "$(dirname "$0")/../.." && pwd -W)"
TRACES="${2:-$(ls -d "$ROOT"/.work/re/*/ | sed 's|/$||' | paste -sd+)}"
PROJ="${3:-dbz_trace}"
GHIDRA="$(cygpath -w "$GHIDRA_HOME")\support\analyzeHeadless.bat"

if [ -n "${GHIDRA_SETTINGS:-}" ]; then
  export GHIDRA_HEADLESS_JAVA_OPTIONS="-Dapplication.settingsdir=$(cygpath -w "$GHIDRA_SETTINGS")"
fi

# cmd.exe cannot handle parentheses in the ROM file name: import from a temporary copy.
TMP_ROM="$(cygpath -u "$TEMP")/dbz_import.sfc"
cp "$ROM" "$TMP_ROM"
trap 'rm -f "$TMP_ROM"' EXIT

mkdir -p "$GHIDRA_PROJECTS"
rm -rf "$GHIDRA_PROJECTS/$PROJ.gpr" "$GHIDRA_PROJECTS/$PROJ.rep"

APPLY=(-preScript DbzApplyTrace.java "$TRACES")
if [ "${DBZ_BASELINE:-0}" = "1" ]; then
  APPLY=(-preScript DbzVectors.java)
fi

echo | cmd //c "$GHIDRA" "$(cygpath -w "$GHIDRA_PROJECTS")" "$PROJ" \
  -import "$(cygpath -w "$TMP_ROM")" \
  -scriptPath "$(cygpath -w "$HERE/ghidra_scripts")" \
  -preScript DbzMirrors.java "${APPLY[@]}" \
  -postScript DbzInfo.java 2>&1 \
  | grep -E "\.java> |REPORT: (Import|Save)|ERROR [^c]" | grep -vE "_(rom|mirror|lorom) " | sed 's/.*\.java> //; s/ (GhidraScript)//'
