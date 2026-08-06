$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$tauriRoot = Join-Path $repoRoot 'src-tauri'
$releaseRoot = Join-Path $repoRoot 'release'

Push-Location $tauriRoot
try {
    $metadata = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        throw 'cargo build --release failed'
    }
    $sourceExe = Join-Path $metadata.target_directory 'release\qq-farm-code-helper.exe'
    if (-not (Test-Path -LiteralPath $sourceExe)) {
        throw "Release executable not found: $sourceExe"
    }
    New-Item -ItemType Directory -Path $releaseRoot -Force | Out-Null
    Copy-Item -LiteralPath $sourceExe -Destination (Join-Path $releaseRoot 'QQFarmCodeHelper.exe') -Force
}
finally {
    Pop-Location
}

Write-Host "Built: $releaseRoot\QQFarmCodeHelper.exe"
