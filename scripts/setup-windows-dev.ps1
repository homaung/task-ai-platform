param(
    [switch]$SkipNativeTools,
    [switch]$SkipDependencies
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$projectRoot = Split-Path -Parent $PSScriptRoot
$machineRoot = Join-Path $env:LOCALAPPDATA 'TaskAIPlatform'
$toolsRoot = Join-Path $machineRoot 'tools'
$downloadsRoot = Join-Path $machineRoot 'downloads'
$nodeVersion = '24.18.0'
$nodeFolder = "node-v$nodeVersion-win-x64"
$nodeRoot = Join-Path $toolsRoot $nodeFolder
$pnpmRoot = Join-Path $toolsRoot 'pnpm'
$cargoHome = Join-Path $toolsRoot 'cargo'
$rustupHome = Join-Path $toolsRoot 'rustup'
$rustToolchain = 'nightly-2025-12-04'

New-Item -ItemType Directory -Force -Path $toolsRoot, $downloadsRoot | Out-Null

function Write-Step([string]$Message) {
    Write-Host "`n==> $Message" -ForegroundColor Cyan
}

function Require-Winget {
    if (-not (Get-Command winget.exe -ErrorAction SilentlyContinue)) {
        throw 'winget was not found. Install App Installer from Microsoft Store and retry.'
    }
}

function Install-WingetPackage {
    param(
        [Parameter(Mandatory)] [string]$Id,
        [string]$Override
    )

    Require-Winget
    $arguments = @(
        'install', '--id', $Id, '--exact', '--silent',
        '--accept-package-agreements', '--accept-source-agreements'
    )
    if ($Override) {
        $arguments += @('--override', $Override)
    }
    & winget.exe @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to install $Id."
    }
}

Write-Step 'Checking Git'
if (-not (Get-Command git.exe -ErrorAction SilentlyContinue)) {
    Install-WingetPackage -Id 'Git.Git'
    $env:PATH = "${env:ProgramFiles}\Git\cmd;$env:PATH"
}

Write-Step "Preparing Node.js $nodeVersion"
if (-not (Test-Path -LiteralPath (Join-Path $nodeRoot 'node.exe'))) {
    $nodeZip = Join-Path $downloadsRoot "$nodeFolder.zip"
    if (-not (Test-Path -LiteralPath $nodeZip)) {
        Invoke-WebRequest -Uri "https://nodejs.org/dist/v$nodeVersion/$nodeFolder.zip" -OutFile $nodeZip
    }
    Expand-Archive -LiteralPath $nodeZip -DestinationPath $toolsRoot -Force
}
$env:PATH = "$nodeRoot;$env:PATH"

Write-Step 'Preparing pnpm 10.13.1'
New-Item -ItemType Directory -Force -Path $pnpmRoot | Out-Null
$pnpmCommand = Join-Path $pnpmRoot 'pnpm.cmd'
if (-not (Test-Path -LiteralPath $pnpmCommand)) {
    & (Join-Path $nodeRoot 'npm.cmd') install --global --prefix $pnpmRoot 'pnpm@10.13.1'
    if ($LASTEXITCODE -ne 0) {
        throw 'Failed to install pnpm.'
    }
}
$env:PATH = "$pnpmRoot;$env:PATH"

Write-Step "Preparing Rust $rustToolchain"
$env:CARGO_HOME = $cargoHome
$env:RUSTUP_HOME = $rustupHome
$rustupCommand = Join-Path $cargoHome 'bin\rustup.exe'
if (-not (Test-Path -LiteralPath $rustupCommand)) {
    $rustupInstaller = Join-Path $downloadsRoot 'rustup-init.exe'
    if (-not (Test-Path -LiteralPath $rustupInstaller)) {
        Invoke-WebRequest -Uri 'https://win.rustup.rs/x86_64' -OutFile $rustupInstaller
    }
    & $rustupInstaller -y --no-modify-path --default-toolchain none
    if ($LASTEXITCODE -ne 0) {
        throw 'Failed to install rustup.'
    }
}
$env:PATH = "$(Join-Path $cargoHome 'bin');$env:PATH"
& $rustupCommand toolchain install $rustToolchain --profile default --component rustfmt rust-analyzer rust-src
if ($LASTEXITCODE -ne 0) {
    throw 'Failed to install the Rust toolchain.'
}
& $rustupCommand default $rustToolchain

if (-not $SkipNativeTools) {
    Write-Step 'Checking Windows C++ Build Tools'
    $vsWhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $hasBuildTools = $false
    if (Test-Path -LiteralPath $vsWhere) {
        $installationPath = & $vsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        $hasBuildTools = -not [string]::IsNullOrWhiteSpace($installationPath)
    }
    if (-not $hasBuildTools) {
        Install-WingetPackage -Id 'Microsoft.VisualStudio.2022.BuildTools' -Override '--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
    }

    Write-Step 'Checking LLVM/libclang'
    $libclang = Join-Path $env:ProgramFiles 'LLVM\bin\libclang.dll'
    if (-not (Test-Path -LiteralPath $libclang)) {
        Install-WingetPackage -Id 'LLVM.LLVM'
    }
}

. (Join-Path $PSScriptRoot 'windows-dev-env.ps1')

if (-not $SkipDependencies) {
    Write-Step 'Installing JavaScript dependencies'
    Push-Location $projectRoot
    try {
        & $pnpmCommand install --frozen-lockfile
        if ($LASTEXITCODE -ne 0) {
            throw 'pnpm install failed.'
        }

        Write-Step 'Downloading Rust dependencies'
        & (Join-Path $env:CARGO_HOME 'bin\cargo.exe') fetch --locked
        if ($LASTEXITCODE -ne 0) {
            throw 'cargo fetch failed.'
        }
    }
    finally {
        Pop-Location
    }
}

Write-Host "`nDevelopment environment is ready." -ForegroundColor Green
Write-Host 'Use "Open Development Shell.cmd" from now on.'

