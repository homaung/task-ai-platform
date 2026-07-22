$ErrorActionPreference = 'Stop'

$script:TaskAiProjectRoot = Split-Path -Parent $PSScriptRoot
$script:TaskAiMachineRoot = Join-Path $env:LOCALAPPDATA 'TaskAIPlatform'
$script:TaskAiToolsRoot = Join-Path $script:TaskAiMachineRoot 'tools'

$nodeRoot = Join-Path $script:TaskAiToolsRoot 'node-v24.18.0-win-x64'
$pnpmRoot = Join-Path $script:TaskAiToolsRoot 'pnpm'
$cargoHome = Join-Path $script:TaskAiToolsRoot 'cargo'
$rustupHome = Join-Path $script:TaskAiToolsRoot 'rustup'
$cargoBin = Join-Path $cargoHome 'bin'

if (-not (Test-Path -LiteralPath (Join-Path $nodeRoot 'node.exe'))) {
    throw 'Node.js is not ready. Run "Setup Development.cmd" first.'
}

if (-not (Test-Path -LiteralPath (Join-Path $cargoBin 'cargo.exe'))) {
    throw 'Rust is not ready. Run "Setup Development.cmd" first.'
}

$env:CARGO_HOME = $cargoHome
$env:RUSTUP_HOME = $rustupHome
$env:CARGO_TARGET_DIR = Join-Path $script:TaskAiMachineRoot 'build\task-ai-platform\target'
$env:PATH = "$nodeRoot;$pnpmRoot;$cargoBin;$env:PATH"

$llvmCandidates = @(
    $env:LIBCLANG_PATH,
    (Join-Path $env:ProgramFiles 'LLVM\bin'),
    (Join-Path $script:TaskAiToolsRoot 'llvm\bin')
) | Where-Object { $_ -and (Test-Path -LiteralPath (Join-Path $_ 'libclang.dll')) }

if (@($llvmCandidates).Count -gt 0) {
    $env:LIBCLANG_PATH = @($llvmCandidates)[0]
}

