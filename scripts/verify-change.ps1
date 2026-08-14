param(
    [ValidateSet('quick', 'normal', 'release')]
    [string]$Level = 'quick'
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    $changed = @(
        git -c core.safecrlf=false diff --name-only --diff-filter=ACMR
        git -c core.safecrlf=false diff --cached --name-only --diff-filter=ACMR
    )
    $untracked = @(git -c core.safecrlf=false ls-files --others --exclude-standard | Where-Object {
        $_ -and -not $_.StartsWith('.ai-room/') -and -not $_.StartsWith('.cargo-cache/') -and -not $_.StartsWith('.cargo-target/')
    })
    $files = @($changed + $untracked | Sort-Object -Unique | Where-Object {
        $_ -and -not $_.StartsWith('.ai-room/') -and -not $_.StartsWith('.cargo-cache/') -and -not $_.StartsWith('.cargo-target/')
    })

    if ($files.Count -eq 0) {
        Write-Host 'No changed files to verify.' -ForegroundColor Green
        exit 0
    }

    git -c core.safecrlf=false diff --check -- @files
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    git -c core.safecrlf=false diff --cached --check -- @files
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    $untrackedWhitespace = @()
    foreach ($file in $untracked) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { continue }
        $matches = @(Select-String -LiteralPath $file -Pattern '[ \t]+$')
        foreach ($match in $matches) {
            $untrackedWhitespace += "${file}:$($match.LineNumber): trailing whitespace."
        }
    }
    if ($untrackedWhitespace.Count -gt 0) {
        $untrackedWhitespace | Write-Error
        exit 1
    }

    $rustFiles = @($files | Where-Object { $_ -match '\.(rs|toml)$' -or $_ -match '^crates/db/migrations/.+\.sql$' })
    $webFiles = @($files | Where-Object { $_ -match '^packages/web-core/.+\.(ts|tsx|js|jsx|json|css|md)$' })

    if ($rustFiles.Count -gt 0) {
        . (Join-Path $PSScriptRoot 'windows-dev-env.ps1')
        cargo fmt --all -- --check
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    if ($webFiles.Count -gt 0) {
        . (Join-Path $PSScriptRoot 'windows-dev-env.ps1')
        & (Join-Path $projectRoot 'packages\web-core\node_modules\.bin\prettier.CMD') --check @webFiles
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    if ($Level -eq 'quick') {
        Write-Host 'Quick verification passed.' -ForegroundColor Green
        exit 0
    }

    . (Join-Path $PSScriptRoot 'windows-dev-env.ps1')
    pnpm run generate-types:check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    pnpm --filter @vibe/web-core run check
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    cargo check -p server -p db
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    if ($Level -eq 'normal') {
        Write-Host 'Normal verification passed.' -ForegroundColor Green
        exit 0
    }

    cargo test --workspace
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    pnpm run tauri:build
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host 'Release verification passed.' -ForegroundColor Green
}
finally {
    Pop-Location
}
