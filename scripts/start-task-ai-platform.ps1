$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSScriptRoot
$serverPath = Join-Path $projectRoot 'target\debug\server.exe'
$url = 'http://127.0.0.1:3010'
$healthUrl = "$url/api/health"
$logDirectory = Join-Path $projectRoot 'dev_assets\launcher-logs'

function Test-TaskAiServer {
    try {
        $response = Invoke-RestMethod -Uri $healthUrl -TimeoutSec 2
        return $response.success -eq $true
    }
    catch {
        return $false
    }
}

if (-not (Test-TaskAiServer)) {
    if (-not (Test-Path -LiteralPath $serverPath)) {
        Add-Type -AssemblyName PresentationFramework
        [System.Windows.MessageBox]::Show(
            "서버 실행 파일을 찾을 수 없습니다.`n$serverPath",
            'Task AI Platform'
        ) | Out-Null
        exit 1
    }

    New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null

    $env:HOST = '127.0.0.1'
    $env:BACKEND_PORT = '3010'
    $env:PREVIEW_PROXY_PORT = '3011'

    Start-Process `
        -FilePath $serverPath `
        -WorkingDirectory $projectRoot `
        -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $logDirectory 'server-out.log') `
        -RedirectStandardError (Join-Path $logDirectory 'server-error.log')

    $ready = $false
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Milliseconds 500
        if (Test-TaskAiServer) {
            $ready = $true
            break
        }
    }

    if (-not $ready) {
        Add-Type -AssemblyName PresentationFramework
        [System.Windows.MessageBox]::Show(
            "로컬 서버를 시작하지 못했습니다.`n로그: $logDirectory",
            'Task AI Platform'
        ) | Out-Null
        exit 1
    }
}

Start-Process $url
