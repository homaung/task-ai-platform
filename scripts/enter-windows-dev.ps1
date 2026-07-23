$ErrorActionPreference = 'Stop'

. (Join-Path $PSScriptRoot 'windows-dev-env.ps1')
Set-Location -LiteralPath $script:TaskAiProjectRoot

Write-Host 'Task AI Platform development shell' -ForegroundColor Green
Write-Host "Project: $script:TaskAiProjectRoot"
Write-Host "Build cache: $env:CARGO_TARGET_DIR"
Write-Host ''
Write-Host 'Common commands:' -ForegroundColor Cyan
Write-Host '  pnpm run dev'
Write-Host '  pnpm --filter @vibe/local-web check'
Write-Host '  cargo test -p server ssh_hosts --lib'
Write-Host '  cargo build -p task-ai-platform --release'
