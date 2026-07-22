param(
    [Parameter(Mandatory)]
    [ValidateSet('Open', 'Save', 'Run')]
    [string]$Action,

    [Parameter(Mandatory)]
    [string]$DriveRoot,

    [string]$LocalRepository = (Join-Path $env:USERPROFILE 'AI-Workspace\task-ai-platform'),

    [switch]$NoLaunch
)

$ErrorActionPreference = 'Stop'
$DriveRoot = (Resolve-Path -LiteralPath $DriveRoot).Path
$repository = Join-Path $DriveRoot 'repository.git'
$runtime = Join-Path $DriveRoot 'runtime\Task AI Platform.exe'
$localRepository = [System.IO.Path]::GetFullPath($LocalRepository)
$localParent = Split-Path -Parent $localRepository

function Show-Message([string]$Text, [string]$Title = 'Task AI Platform') {
    Add-Type -AssemblyName PresentationFramework
    [System.Windows.MessageBox]::Show($Text, $Title) | Out-Null
}

function Require-Git {
    if (-not (Get-Command git.exe -ErrorAction SilentlyContinue)) {
        Show-Message 'Git is required. Run: winget install --id Git.Git'
        throw 'Git is not installed.'
    }
}

function Invoke-Git {
    param([Parameter(ValueFromRemainingArguments)] [string[]]$Arguments)
    & git.exe -C $localRepository @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Git command failed: git $($Arguments -join ' ')"
    }
}

if ($Action -eq 'Run') {
    if (-not (Test-Path -LiteralPath $runtime)) {
        Show-Message "Application executable was not found.`n$runtime"
        exit 1
    }
    Start-Process -FilePath $runtime -WorkingDirectory (Split-Path -Parent $runtime)
    exit 0
}

Require-Git
if (-not (Test-Path -LiteralPath $repository)) {
    Show-Message "Drive repository was not found.`n$repository"
    exit 1
}

if ($Action -eq 'Open') {
    if (-not (Test-Path -LiteralPath (Join-Path $localRepository '.git'))) {
        New-Item -ItemType Directory -Force -Path $localParent | Out-Null
        & git.exe clone $repository $localRepository
        if ($LASTEXITCODE -ne 0) {
            throw 'Failed to create the local working copy.'
        }
    }
    else {
        $dirty = & git.exe -C $localRepository status --porcelain
        if ($dirty) {
            Show-Message "Local changes have not been saved, so automatic update was skipped.`nRun Save Workspace.cmd first."
        }
        else {
            Invoke-Git fetch origin main
            Invoke-Git merge --ff-only origin/main
        }
    }

    if (-not $NoLaunch) {
        Start-Process explorer.exe -ArgumentList $localRepository
        $shellArguments = @(
            '-NoLogo',
            '-NoExit',
            '-Command',
            "Set-Location -LiteralPath '$($localRepository.Replace("'", "''"))'"
        )
        Start-Process powershell.exe -WorkingDirectory $localRepository -ArgumentList $shellArguments
    }
    exit 0
}

if (-not (Test-Path -LiteralPath (Join-Path $localRepository '.git'))) {
    Show-Message "Local working copy does not exist.`nRun Open Workspace.cmd first."
    exit 1
}

$status = & git.exe -C $localRepository status --short
if (-not $status) {
    Show-Message 'There are no changes to save.'
    exit 0
}

Write-Host 'Changes to save to Drive:' -ForegroundColor Cyan
$status | ForEach-Object { Write-Host $_ }
$answer = Read-Host 'Save these changes to Drive? (y/N)'
if ($answer -notin @('y', 'Y', 'yes', 'YES')) {
    exit 0
}

$userName = & git.exe -C $localRepository config user.name
if ([string]::IsNullOrWhiteSpace($userName)) {
    $userName = Read-Host 'Git user name'
    Invoke-Git config user.name $userName
}
$userEmail = & git.exe -C $localRepository config user.email
if ([string]::IsNullOrWhiteSpace($userEmail)) {
    $userEmail = Read-Host 'Git user email'
    Invoke-Git config user.email $userEmail
}

$message = Read-Host 'Commit message (Enter=automatic timestamp)'
if ([string]::IsNullOrWhiteSpace($message)) {
    $message = "WIP sync $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
}

Invoke-Git add -A
Invoke-Git commit -m $message
Invoke-Git pull --rebase origin main
Invoke-Git push origin main

Show-Message "Saved to Drive.`nWait for Google Drive sync before opening the workspace on another PC."
