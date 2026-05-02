# Wave 4.4 smoke runner -- Windows (PowerShell).
#
# Mirrors scripts/smoke-test.sh: launches tynd-{full,lite}.exe --smoke-mode
# against the minimal app under scripts/smoke-test-app and asserts the
# process exits cleanly within 10 s.
#
# NOTE: --smoke-mode is NOT yet wired in the host Rust binaries (see W4.4
# log entry in IMPLEMENTATION_PROGRESS.md). Until that lands, smoke.yml
# only invokes this script on `workflow_dispatch`.

param(
    [string]$Runtime = "full"
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$AppDir = Join-Path $ScriptDir "smoke-test-app"
$Bin = "./target/release/tynd-$Runtime.exe"

# Tag prefix for log lines -- kept as a single-quoted literal because PS
# parses unescaped square brackets inside double-quoted strings as array
# index syntax (e.g. "[smoke]" trips a parser error).
$Tag = '[smoke]'

Write-Host "$Tag runtime=$Runtime bin=$Bin app=$AppDir"

if (-not (Test-Path $Bin)) {
    Write-Error "$Tag ERROR: $Bin not found"
    exit 1
}

Push-Location $AppDir
try {
    $proc = Start-Process -FilePath $Bin -ArgumentList "--smoke-mode" -PassThru -NoNewWindow
    $exited = $proc.WaitForExit(10000)
    if (-not $exited) {
        $proc.Kill()
        Write-Error "$Tag ERROR: TIMED OUT -- window did not close within 10s"
        exit 1
    }
    if ($proc.ExitCode -ne 0) {
        Write-Error "$Tag ERROR: FAILED with exit code $($proc.ExitCode)"
        exit $proc.ExitCode
    }
    Write-Host "$Tag PASSED for $Runtime"
}
finally {
    Pop-Location
}
