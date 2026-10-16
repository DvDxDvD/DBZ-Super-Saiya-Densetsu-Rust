$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureName = ".assemble-test-$([guid]::NewGuid().ToString('N'))"
$fixture = Join-Path $root $fixtureName
$outDir = Join-Path $fixture 'output'
$relativeOutDir = Join-Path $fixtureName 'output'
$crate = Join-Path $root 'crates/ssd-app'
$assemble = Join-Path $PSScriptRoot 'assemble.ps1'

function Quote-PS([string]$value) { "'" + $value.Replace("'", "''") + "'" }

if (-not (Test-Path -LiteralPath $crate -PathType Container)) { throw "Missing crate directory: $crate" }
if (Test-Path -LiteralPath $fixture) { throw "Fixture path already exists: $fixture" }

try {
    New-Item -ItemType Directory -Path $fixture | Out-Null
    $exe = Join-Path $fixture 'dummy.exe'
    [System.IO.File]::WriteAllBytes($exe, [byte[]](0))

    $command = "Set-Location -LiteralPath $(Quote-PS $root); & $(Quote-PS $assemble) -Exe $(Quote-PS $exe) -OutDir $(Quote-PS $relativeOutDir)"
    $start = [System.Diagnostics.ProcessStartInfo]::new((Get-Command pwsh).Source)
    $start.WorkingDirectory = $crate
    $start.ArgumentList.Add('-NoProfile')
    $start.ArgumentList.Add('-EncodedCommand')
    $start.ArgumentList.Add([Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command)))
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::Start($start)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()

    if ($process.ExitCode -ne 0) { throw "assemble.ps1 exited $($process.ExitCode):`n$stdout`n$stderr" }
    foreach ($path in @((Join-Path $outDir 'QUICKSTART.txt'), (Join-Path $outDir 'ssd-recompiled.exe'))) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing assembled file: $path" }
    }
    Write-Host 'PASS: relative OutDir resolves from PowerShell location when process cwd differs.'
}
finally {
    if ($fixture.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -and
        $fixtureName -match '^\.assemble-test-[0-9a-f]{32}$' -and
        (Test-Path -LiteralPath $fixture)) {
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
