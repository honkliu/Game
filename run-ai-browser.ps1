# Build and run the AI-native browser (one executable; the AI utility runs inside it as `browser --ai-utility`).
$ErrorActionPreference = 'Stop'
$env:CARGO_INCREMENTAL = '0'
Push-Location $PSScriptRoot\ai-browser
try { cargo run --release } finally { Pop-Location }
