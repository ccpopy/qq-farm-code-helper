$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location (Join-Path $repoRoot 'src-tauri')
try {
    cargo run
}
finally {
    Pop-Location
}
