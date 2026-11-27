# Launches VIctory with the switches this branch adds, as used on an AMD RX 6800 (16 GB, ReBAR on).
# Usage: powershell -ExecutionPolicy Bypass -File scripts\play-rx6800.ps1 -Exe <path to the title's executable> [-Health]
# Build first: cargo build --release -p victory
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Victory = (Join-Path $PSScriptRoot '..\target\release\victory.exe'),
    [switch]$Health
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path $Victory)) { throw "victory.exe not found at $Victory; run: cargo build --release -p victory" }
if (-not (Test-Path $Exe)) { throw "the title's executable not found at $Exe" }

# Refreshes pass over 2 MiB chunks with no write since (fxceless/victory-fork#1358).
$env:VICTORY_COLD_SKIP = '1'
# Plain copies of card ranges (already on main).
$env:VICTORY_COPY_PLAIN = '1'
# Optional: one CSV row per frame of work not done and VRAM use (fxceless/victory-fork#1415).
if ($Health) { $env:VICTORY_FRAME_HEALTH = (Join-Path (Get-Location) 'frame-health.csv') }
# The frame-rate box in the corner is main's own (VICTORY_FPS_COUNTER, on by default).

& $Victory run $Exe --execute
exit $LASTEXITCODE
