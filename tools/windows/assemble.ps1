<#
.SYNOPSIS
  Lays out the Windows folder of the game: the program, the launchers, the quickstart and the licences.
.DESCRIPTION
  Used by build-classic.ps1 and build-recompiled.ps1. The folder is what the prebuilt download holds:
  ssd-recompiled.exe, symbols.toml, "Play Classic.bat", "Play Remastered.bat", "Set up my ROM.bat",
  QUICKSTART.txt, LICENSE, NOTICE.md and licenses\. hd-pack.zip is a separate download that the player
  puts beside the program.
.PARAMETER Exe
  The built ssd-recompiled.exe.
.PARAMETER OutDir
  The folder to make (it is emptied first).
.PARAMETER Hooks
  Extra arguments of the launchers: '--hooks rust' for a build with the recompiled code, nothing otherwise.
.PARAMETER Zip
  Also make <OutDir>.zip.
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$OutDir,
    [string]$Hooks = '',
    [switch]$Zip
)
$ErrorActionPreference = 'Stop'
$OutDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutDir)
$here = $PSScriptRoot
$root = Split-Path -Parent (Split-Path -Parent $here)

if (-not (Test-Path -LiteralPath $Exe)) { throw "The program is not there: $Exe" }
if (Test-Path -LiteralPath $OutDir) { Remove-Item -Recurse -Force -LiteralPath $OutDir }
New-Item -ItemType Directory -Force -Path (Join-Path $OutDir 'licenses') | Out-Null

function Write-Crlf([string]$Path, [string]$Text) {
    $crlf = $Text -replace "`r?`n", "`r`n"
    [System.IO.File]::WriteAllText($Path, $crlf, (New-Object System.Text.UTF8Encoding($false)))
}

Copy-Item -LiteralPath $Exe -Destination (Join-Path $OutDir 'ssd-recompiled.exe')
Copy-Item -LiteralPath (Join-Path $root 're/symbols.toml') -Destination (Join-Path $OutDir 'symbols.toml')

foreach ($name in 'Play Classic.bat', 'Play Remastered.bat', 'Set up my ROM.bat') {
    $text = (Get-Content -Raw -LiteralPath (Join-Path $here $name)).Replace('@HOOKS@', $Hooks)
    Write-Crlf (Join-Path $OutDir $name) $text
}
Write-Crlf (Join-Path $OutDir 'QUICKSTART.txt') (Get-Content -Raw -LiteralPath (Join-Path $here 'QUICKSTART.txt'))

# Licence texts: the project's own, then the third-party ones
$licence = @('LICENSE', 'tools/public/LICENSE') | ForEach-Object { Join-Path $root $_ } |
    Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
Copy-Item -LiteralPath $licence -Destination (Join-Path $OutDir 'LICENSE.txt')
$notice = @('NOTICE.md', 'tools/public/NOTICE.md') | ForEach-Object { Join-Path $root $_ } |
    Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
Copy-Item -LiteralPath $notice -Destination (Join-Path $OutDir 'NOTICE.md')
Copy-Item -LiteralPath (Join-Path $root 'third_party/jgenesis/LICENSE') `
    -Destination (Join-Path $OutDir 'licenses/jgenesis-GPL-3.0.txt')
Get-ChildItem -LiteralPath (Join-Path $root 'assets/fonts') -Filter '*-OFL.txt' |
    Copy-Item -Destination (Join-Path $OutDir 'licenses')

if ($Zip) {
    $archive = "$OutDir.zip"
    if (Test-Path -LiteralPath $archive) { Remove-Item -Force -LiteralPath $archive }
    Compress-Archive -Path (Join-Path $OutDir '*') -DestinationPath $archive
    Write-Host "Wrote $archive"
}
Write-Host "Laid out $OutDir"
