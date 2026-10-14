<#
.SYNOPSIS
  Builds the game without the recompiled code and lays out the Windows folder (the prebuilt download).
.DESCRIPTION
  The Classic build: the original game runs in the emulator core, and the routines ported by hand
  (re/symbols.toml) are there for --hooks rust. It needs nothing but the Rust toolchain (rustup installs
  the pinned one), CMake and the MSVC build tools (SDL3 is built from source). No ROM is needed to build.
.PARAMETER OutDir
  Where to lay out the folder [default: dist/dbz-ssd-recompiled-windows].
.PARAMETER Zip
  Also make a zip of the folder.
#>
param(
    [string]$OutDir = 'dist/dbz-ssd-recompiled-windows',
    [switch]$Zip
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

cargo build --release -p ssd-app
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

& (Join-Path $PSScriptRoot 'windows/assemble.ps1') -Exe 'target/release/ssd-recompiled.exe' `
    -OutDir $OutDir -Hooks '' -Zip:$Zip
