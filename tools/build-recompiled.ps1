<#
.SYNOPSIS
  Recompiles the game from your ROM and builds it with the recompiled code.
.DESCRIPTION
  The recompiled build: the recompiler (crates/ssd-recomp) turns the 65816 code of YOUR ROM into Rust,
  written to .work/recomp/ (never tracked, never shared: it is derived from the game), and the game is
  built with the `recomp` feature so that code runs instead of the interpreted original. The launchers
  of the folder start the game with --hooks rust. It takes a while (the generated source is about 24 MB;
  expect ten minutes or more to compile).
  Needs the Rust toolchain, CMake and the MSVC build tools, and your ROM: give it with -Rom the first
  time (the game keeps a clean copy under %LOCALAPPDATA%\dbz-ssd-recompiled\ and reads it from there
  afterwards).
.PARAMETER Rom
  Your ROM file (English with the Klepto v1.02 translation, or the Japanese Rev 1 ROM with -Ips).
.PARAMETER Ips
  The Klepto v1.02 patch (dbz102.ips), for the Japanese Rev 1 ROM.
.PARAMETER OutDir
  Where to lay out the folder [default: dist/dbz-ssd-recompiled-windows-recompiled].
.PARAMETER Zip
  Also make a zip of the folder.
#>
param(
    [string]$Rom = '',
    [string]$Ips = '',
    [string]$OutDir = 'dist/dbz-ssd-recompiled-windows-recompiled',
    [switch]$Zip
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Check($what) { if ($LASTEXITCODE -ne 0) { throw "$what failed" } }

$data = if ($env:SSD_DATA_DIR) { $env:SSD_DATA_DIR } else { Join-Path $env:LOCALAPPDATA 'dbz-ssd-recompiled' }
$cache = Join-Path $data 'cache/rom/canonical.sfc'

# 1. The tools, and your ROM in the cache
cargo build --release -p ssd-app -p xtask
Check 'cargo build'
if ($Rom -ne '') {
    $arguments = @('--rom', $Rom, '--headless-frames', '1')
    if ($Ips -ne '') { $arguments += @('--ips', $Ips) }
    & 'target/release/ssd-recompiled.exe' @arguments
    Check 'accepting the ROM'
}
if (-not (Test-Path -LiteralPath $cache)) {
    throw "No ROM yet: run this script again with -Rom <your rom> (and -Ips <dbz102.ips> for the Japanese ROM)."
}

# 2. The recompiler: your ROM to Rust, under .work/recomp/
cargo run --release -p xtask -- recompile
Check 'the recompiler'

# 3. The game, with the generated code
cargo build --release -p ssd-app --features recomp
Check 'cargo build --features recomp'

& (Join-Path $PSScriptRoot 'windows/assemble.ps1') -Exe 'target/release/ssd-recompiled.exe' `
    -OutDir $OutDir -Hooks '--hooks rust' -Zip:$Zip
Write-Host 'The recompiled build is for your own use: the generated code is derived from your ROM, do not share it.'
